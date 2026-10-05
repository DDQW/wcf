//! Live watch rooms (port of HQTube's `Room` + cytu.be-style playlist).
//!
//! One in-memory instance per active room holds the server-authoritative
//! playback clock, the scheduled show, the leader and voteskips. A single
//! ticker drives all rooms once a second (end detection, show transitions,
//! drift-correction rebroadcasts); a scheduler wakes rooms for due shows.
//! Clients talk to it through the realtime hub (`Topic::Watch`, module
//! "watch").

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration as StdDuration;

use serde_json::Value;
use shared::watch::{
    ControlAction, EventStatus, PlayKind, PlaybackState, PlayingMedia, PlaylistSource, PresenceUser, RoomRole,
    ShowInfo, ShowItem, ShowPhase, ShowSummary, SourceKind, WatchClientMsg, WatchEvent, WatchYou, CHAT_HISTORY,
    MODULE, PRESHOW_LEAD_MS,
};
use shared::ws::{ServerMsg, Topic};
use shared::AuthUser;
use sqlx::Row;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::hub::ConnId;
use crate::state::AppState;
use crate::watch::{self, ModEffect, PlaylistRow, RoomRow};
use crate::watch_engine::{self, Cursor, CursorPhase, EngineItem, Recovery};

/// Flood control: at most CHAT_BURST messages per CHAT_WINDOW_MS per user.
const CHAT_BURST: usize = 5;
const CHAT_WINDOW_MS: i64 = 10_000;
/// Grace past the end of an item before advancing.
const END_SLACK_MS: f64 = 2000.0;
/// How long the "that's a wrap" card stays up after a show.
const ENDED_CARD_MS: i64 = 10 * 60_000;
const MAX_INTERMISSION_SHIFT_MS: i64 = 30 * 60_000;
/// Re-broadcast the state every N ticks for drift correction.
const STATE_REBROADCAST_TICKS: u64 = 5;
/// Empty rooms without a show are dropped from memory after this.
const DISPOSE_AFTER_MS: i64 = 60_000;
const MAX_DURATION_MS: i64 = 24 * 3_600_000;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn now_f() -> f64 {
    crate::realtime::now_ms()
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Summary {
    now_playing: Option<String>,
    show: Option<ShowSummary>,
    stage_media: Option<Uuid>,
}

#[derive(Default)]
pub struct WatchLive {
    rooms: StdMutex<HashMap<Uuid, Arc<Mutex<LiveRoom>>>>,
    summaries: StdMutex<HashMap<Uuid, Summary>>,
    scheduler: tokio::sync::Notify,
}

impl WatchLive {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone)]
enum Phase {
    Preshow,
    Item,
    Intermission { ends_at_ms: i64 },
}

#[derive(Debug, Clone)]
struct LiveShow {
    event_id: Uuid,
    title: String,
    starts_at_ms: i64,
    /// Mirrors `watch_events.status` — false while still `scheduled`.
    live: bool,
    items: Vec<ShowItem>,
    /// Current item; -1 during preshow.
    index: i64,
    phase: Phase,
}

struct LiveRoom {
    row: RoomRow,
    media: Option<PlayingMedia>,
    base_pos_ms: f64,
    base_at_ms: f64,
    playing: bool,
    rate: f64,
    show: Option<LiveShow>,
    ended_card: Option<(Uuid, String, i64)>,
    /// cytu.be leader: may drive playback without being a moderator.
    leader: Option<Uuid>,
    votes: HashSet<Uuid>,
    last_chat: HashMap<Uuid, i64>,
    chat_stamps: HashMap<Uuid, VecDeque<i64>>,
    muted_until: HashMap<Uuid, i64>,
    roles: HashMap<Uuid, Option<RoomRole>>,
    adopting: bool,
    tick_count: u64,
    empty_since: Option<i64>,
}

impl LiveRoom {
    fn new(row: RoomRow) -> Self {
        Self {
            row,
            media: None,
            base_pos_ms: 0.0,
            base_at_ms: 0.0,
            playing: false,
            rate: 1.0,
            show: None,
            ended_card: None,
            leader: None,
            votes: HashSet::new(),
            last_chat: HashMap::new(),
            chat_stamps: HashMap::new(),
            muted_until: HashMap::new(),
            roles: HashMap::new(),
            adopting: false,
            tick_count: 0,
            empty_since: None,
        }
    }

    fn id(&self) -> Uuid {
        self.row.id
    }

    /// Unclamped position (end-of-item detection may exceed the duration).
    fn raw_pos_ms(&self) -> f64 {
        if self.media.is_none() {
            return 0.0;
        }
        if self.playing {
            self.base_pos_ms + (now_f() - self.base_at_ms) * self.rate
        } else {
            self.base_pos_ms
        }
    }

    fn current_pos_ms(&self) -> f64 {
        let pos = self.raw_pos_ms();
        match self.media.as_ref().and_then(|m| m.duration_ms) {
            Some(d) => pos.min(d as f64),
            None => pos,
        }
    }

    fn snapshot(&self) -> PlaybackState {
        if let Some(media) = &self.media {
            return if self.playing {
                PlaybackState::Playing {
                    media: media.clone(),
                    position_ms: self.current_pos_ms(),
                    at_server_ms: now_f(),
                    rate: self.rate,
                }
            } else {
                PlaybackState::Paused { media: media.clone(), position_ms: self.base_pos_ms }
            };
        }
        if let Some(s) = &self.show {
            match &s.phase {
                Phase::Preshow => {
                    return PlaybackState::Preshow {
                        event_id: s.event_id,
                        title: s.title.clone(),
                        starts_at_ms: s.starts_at_ms,
                        poster_url: s.items.first().and_then(|i| i.poster_url.clone()),
                    }
                }
                Phase::Intermission { ends_at_ms } => {
                    let next = s.items.get((s.index + 1) as usize);
                    return PlaybackState::Intermission {
                        event_id: s.event_id,
                        ends_at_ms: *ends_at_ms,
                        next_title: next.map(|n| n.title.clone()),
                        next_poster_url: next.and_then(|n| n.poster_url.clone()),
                        item_index: s.index.max(0) as usize,
                        item_count: s.items.len(),
                    };
                }
                Phase::Item => {}
            }
        }
        if let Some((event_id, title, _)) = &self.ended_card {
            return PlaybackState::Ended { event_id: *event_id, title: title.clone() };
        }
        PlaybackState::Idle
    }

    fn show_phase(&self) -> Option<ShowPhase> {
        let s = self.show.as_ref()?;
        Some(match s.phase {
            Phase::Preshow => ShowPhase::Preshow,
            Phase::Intermission { .. } => ShowPhase::Intermission,
            Phase::Item if self.playing => ShowPhase::Playing,
            Phase::Item => ShowPhase::Paused,
        })
    }

    fn show_info(&self) -> Option<ShowInfo> {
        let s = self.show.as_ref()?;
        Some(ShowInfo {
            event_id: s.event_id,
            title: s.title.clone(),
            status: if s.live { EventStatus::Live } else { EventStatus::Scheduled },
            starts_at_ms: s.starts_at_ms,
            items: s.items.clone(),
            current_index: (s.index >= 0).then_some(s.index as usize),
            phase: self.show_phase(),
        })
    }

    fn current_item(&self) -> Option<Uuid> {
        self.media.as_ref().and_then(|m| m.item_id)
    }

    fn summary(&self) -> Summary {
        Summary {
            now_playing: self.media.as_ref().map(|m| m.title.clone()),
            show: self.show.as_ref().map(|s| ShowSummary {
                title: s.title.clone(),
                live: s.live,
                phase: self.show_phase(),
            }),
            stage_media: self.media.as_ref().and_then(|m| m.media_id),
        }
    }

    fn clear_stage(&mut self) {
        self.media = None;
        self.playing = false;
        self.base_pos_ms = 0.0;
        self.votes.clear();
    }

    fn set_stage(&mut self, media: PlayingMedia, pos: f64, paused: bool) {
        self.media = Some(media);
        self.base_pos_ms = pos.max(0.0);
        self.base_at_ms = now_f();
        self.playing = !paused;
        self.votes.clear();
    }
}

fn live(state: &AppState) -> &WatchLive {
    &state.watch
}

fn emit(ev: &WatchEvent) -> ServerMsg {
    ServerMsg::Module {
        module: MODULE.to_string(),
        payload: serde_json::to_value(ev).unwrap_or(Value::Null),
    }
}

fn publish(state: &AppState, room_id: Uuid, ev: &WatchEvent) {
    state.hub.publish(&Topic::Watch(room_id), &emit(ev));
}

fn send_error(state: &AppState, conn: ConnId, room_id: Uuid, code: &str, message: impl Into<String>) {
    state.hub.send_conn(
        conn,
        &emit(&WatchEvent::Error { room_id, code: code.into(), message: message.into() }),
    );
}

async fn get_room(state: &AppState, id: Uuid) -> Option<Arc<Mutex<LiveRoom>>> {
    if let Some(r) = live(state).rooms.lock().expect("rooms").get(&id) {
        return Some(r.clone());
    }
    let row = watch::room_row(&state.db, id).await.ok().flatten()?;
    let mut rooms = live(state).rooms.lock().expect("rooms");
    // Single instance per room even with concurrent loaders.
    Some(rooms.entry(id).or_insert_with(|| Arc::new(Mutex::new(LiveRoom::new(row)))).clone())
}

fn loaded(state: &AppState, id: Uuid) -> Option<Arc<Mutex<LiveRoom>>> {
    live(state).rooms.lock().expect("rooms").get(&id).cloned()
}

fn store_summary(state: &AppState, room: &LiveRoom) {
    live(state).summaries.lock().expect("summaries").insert(room.id(), room.summary());
}

fn broadcast_state(state: &AppState, room: &LiveRoom) {
    publish(state, room.id(), &WatchEvent::State { room_id: room.id(), state: room.snapshot() });
    store_summary(state, room);
}

fn broadcast_show(state: &AppState, room: &LiveRoom) {
    publish(state, room.id(), &WatchEvent::Show { room_id: room.id(), show: room.show_info() });
}

async fn broadcast_playlist(state: &AppState, room: &LiveRoom) {
    let items = watch::playlist(&state.db, room.id()).await.unwrap_or_default();
    publish(
        state,
        room.id(),
        &WatchEvent::Playlist {
            room_id: room.id(),
            items: items.iter().map(|i| i.view()).collect(),
            current_item: room.current_item(),
        },
    );
}

fn after_show_change(state: &AppState, room: &LiveRoom) {
    broadcast_state(state, room);
    broadcast_show(state, room);
}

/// Cheap directory info without locking rooms.
pub fn summary(state: &AppState, room_id: Uuid) -> (Option<String>, Option<ShowSummary>) {
    let s = live(state).summaries.lock().expect("summaries").get(&room_id).cloned().unwrap_or_default();
    (s.now_playing, s.show)
}

/// Whether some room is playing this library item (its files are in use).
pub fn is_media_on_stage(state: &AppState, media_id: Uuid) -> bool {
    live(state)
        .summaries
        .lock()
        .expect("summaries")
        .values()
        .any(|s| s.stage_media == Some(media_id))
}

pub fn scheduler_kick(state: &AppState) {
    live(state).scheduler.notify_one();
}

// ---------------------------------------------------------------------------
// Roles, presence, voteskip
// ---------------------------------------------------------------------------

async fn role(state: &AppState, room: &mut LiveRoom, user_id: Uuid) -> Option<RoomRole> {
    if let Some(r) = room.roles.get(&user_id) {
        return *r;
    }
    let r = watch::role_of(&state.db, &room.row, user_id).await.ok().flatten();
    room.roles.insert(user_id, r);
    r
}

async fn you(state: &AppState, room: &mut LiveRoom, user: &AuthUser) -> WatchYou {
    let r = role(state, room, user.id).await;
    let leader = room.leader == Some(user.id);
    WatchYou {
        role: r,
        can_control: watch::can_control(user, r),
        can_add: watch::can_add(&room.row, user, r, leader),
        leader,
        muted_until_ms: room.muted_until.get(&user.id).copied().filter(|t| *t > now_ms()),
    }
}

async fn presence(state: &AppState, room: &mut LiveRoom) -> Vec<PresenceUser> {
    let users = state.hub.topic_users(&Topic::Watch(room.id()));
    let mut out = Vec::with_capacity(users.len());
    for u in users {
        let r = role(state, room, u.id).await;
        out.push(PresenceUser { leader: room.leader == Some(u.id), role: r, user: u });
    }
    out
}

fn voteskip_numbers(state: &AppState, room: &LiveRoom) -> (usize, usize) {
    let viewers = state.hub.topic_users(&Topic::Watch(room.id())).len().max(1);
    let needed = ((viewers as f64) * room.row.voteskip_percent as f64 / 100.0).ceil().max(1.0) as usize;
    (room.votes.len(), needed)
}

fn broadcast_voteskip(state: &AppState, room: &LiveRoom) {
    let (votes, needed) = voteskip_numbers(state, room);
    publish(state, room.id(), &WatchEvent::Voteskip { room_id: room.id(), votes, needed });
}

async fn broadcast_presence(state: &AppState, room: &mut LiveRoom) {
    let users = presence(state, room).await;
    publish(state, room.id(), &WatchEvent::Presence { room_id: room.id(), users });
}

async fn send_you(state: &AppState, room: &mut LiveRoom, user_id: Uuid) {
    let conns = state.hub.user_conns_in_topic(user_id, &Topic::Watch(room.id()));
    if conns.is_empty() {
        return;
    }
    let Ok(Some(user)) = crate::users::find_auth_user(&state.db, user_id).await else { return };
    let y = you(state, room, &user).await;
    for c in conns {
        state.hub.send_conn(c, &emit(&WatchEvent::You { room_id: room.id(), you: y.clone() }));
    }
}

// ---------------------------------------------------------------------------
// Realtime hooks
// ---------------------------------------------------------------------------

pub async fn can_subscribe(state: &AppState, user: Option<&AuthUser>, room_id: Uuid) -> bool {
    let Some(user) = user else { return false };
    if !state.settings().modules.watch {
        return false;
    }
    watch::accessible(&state.db, user, room_id).await.is_ok()
}

pub async fn on_join(state: &AppState, conn: ConnId, user: Option<&AuthUser>, room_id: Uuid) {
    let Some(user) = user else { return };
    let Some(room_arc) = get_room(state, room_id).await else { return };
    let mut room = room_arc.lock().await;
    room.empty_since = None;
    room.roles.remove(&user.id);
    if let Ok(effect) = watch::effective_for(&state.db, room_id, user.id).await {
        match effect.muted_until_ms {
            Some(t) => room.muted_until.insert(user.id, t),
            None => room.muted_until.remove(&user.id),
        };
    }
    let Ok(view) = watch::room_view(state, user, room_id).await else { return };
    let y = you(state, &mut room, user).await;
    let playlist = watch::playlist(&state.db, room_id).await.unwrap_or_default();
    let chat = watch::chat_history(&state.db, room_id, CHAT_HISTORY).await.unwrap_or_default();
    let users = presence(state, &mut room).await;
    let voteskip = voteskip_numbers(state, &room);
    state.hub.send_conn(
        conn,
        &emit(&WatchEvent::Welcome {
            room: view,
            you: y,
            state: room.snapshot(),
            show: room.show_info(),
            playlist: playlist.iter().map(|p| p.view()).collect(),
            current_item: room.current_item(),
            chat,
            presence: users.clone(),
            voteskip,
        }),
    );
    publish(state, room_id, &WatchEvent::Presence { room_id, users });
    broadcast_voteskip(state, &room);
    watch::touch_room(&state.db, room_id).await;
}

pub async fn on_leave(state: &AppState, room_id: Uuid) {
    let Some(room_arc) = loaded(state, room_id) else { return };
    let mut room = room_arc.lock().await;
    let present: HashSet<Uuid> = state.hub.topic_users(&Topic::Watch(room_id)).iter().map(|u| u.id).collect();
    room.votes.retain(|u| present.contains(u));
    if room.leader.is_some_and(|l| !present.contains(&l)) {
        room.leader = None;
    }
    broadcast_presence(state, &mut room).await;
    broadcast_voteskip(state, &room);
}

pub async fn dispatch(state: &AppState, conn: ConnId, user: Option<&AuthUser>, payload: Value) {
    let Some(user) = user else { return };
    let Ok(msg) = serde_json::from_value::<WatchClientMsg>(payload) else { return };
    let room_id = match &msg {
        WatchClientMsg::Chat { room_id, .. }
        | WatchClientMsg::Control { room_id, .. }
        | WatchClientMsg::Voteskip { room_id }
        | WatchClientMsg::Duration { room_id, .. } => *room_id,
    };
    // Subscribing checked access; only joined clients may act.
    if !state.hub.user_in_topic(user.id, &Topic::Watch(room_id)) {
        return;
    }
    let Some(room_arc) = get_room(state, room_id).await else { return };
    let mut room = room_arc.lock().await;
    let res = match msg {
        WatchClientMsg::Chat { body, .. } => handle_chat(state, &mut room, user, &body).await,
        WatchClientMsg::Control { action, position_ms, media_id, item_id, event_id, delta_ms, user_id, .. } => {
            handle_control(state, &mut room, user, action, position_ms, media_id, item_id, event_id, delta_ms, user_id)
                .await
        }
        WatchClientMsg::Voteskip { .. } => handle_voteskip(state, &mut room, user).await,
        WatchClientMsg::Duration { key, duration_ms, .. } => handle_duration(state, &mut room, &key, duration_ms).await,
    };
    if let Err((code, message)) = res {
        send_error(state, conn, room_id, code, message);
    }
}

type Reply = std::result::Result<(), (&'static str, String)>;

fn deny(code: &'static str, msg: impl Into<String>) -> Reply {
    Err((code, msg.into()))
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

async fn handle_chat(state: &AppState, room: &mut LiveRoom, user: &AuthUser, body: &str) -> Reply {
    let now = now_ms();
    if let Some(until) = room.muted_until.get(&user.id).copied().filter(|t| *t > now) {
        return deny("muted", format!("Du bist noch {} s stummgeschaltet.", (until - now + 999) / 1000));
    }
    let stamps = room.chat_stamps.entry(user.id).or_default();
    while stamps.front().is_some_and(|t| now - t >= CHAT_WINDOW_MS) {
        stamps.pop_front();
    }
    if stamps.len() >= CHAT_BURST {
        return deny("rate_limited", "Etwas langsamer, bitte.");
    }
    let r = role(state, room, user.id).await;
    if room.row.slow_mode_ms > 0 && !watch::can_control(user, r) {
        let last = room.last_chat.get(&user.id).copied().unwrap_or(0);
        let wait = last + room.row.slow_mode_ms - now;
        if wait > 0 {
            return deny(
                "slow_mode",
                format!(
                    "Slow-Mode: eine Nachricht alle {} s – noch {} s.",
                    room.row.slow_mode_ms / 1000,
                    (wait + 999) / 1000
                ),
            );
        }
    }
    if body.trim().is_empty() {
        return Ok(());
    }
    room.chat_stamps.entry(user.id).or_default().push_back(now);
    room.last_chat.insert(user.id, now);
    match watch::insert_chat(&state.db, room.id(), user, body).await {
        Ok(msg) => {
            publish(state, room.id(), &WatchEvent::Chat { room_id: room.id(), msg });
            Ok(())
        }
        Err(e) => deny("error", e.public_message()),
    }
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn handle_control(
    state: &AppState,
    room: &mut LiveRoom,
    user: &AuthUser,
    action: ControlAction,
    position_ms: Option<f64>,
    media_id: Option<Uuid>,
    item_id: Option<Uuid>,
    event_id: Option<Uuid>,
    delta_ms: Option<i64>,
    target_user: Option<Uuid>,
) -> Reply {
    let r = role(state, room, user.id).await;
    let controller = watch::can_control(user, r);
    let leader = room.leader == Some(user.id);
    let transport = matches!(
        action,
        ControlAction::Play | ControlAction::Pause | ControlAction::Seek | ControlAction::Next | ControlAction::PlayItem
    );
    if !(controller || (leader && transport)) {
        return deny("forbidden", "Nur Inhaber, Moderatoren oder der Leader steuern die Wiedergabe.");
    }
    let show_live = room.show.as_ref().is_some_and(|s| s.live);
    match action {
        ControlAction::PlayNow => {
            if show_live {
                return deny("event_live", "Eine Vorstellung läuft – beende sie zuerst in der Konsole.");
            }
            let Some(id) = media_id else { return Ok(()) };
            let Some(media) = crate::watch_media::playing_media(&state.db, id).await.ok().flatten() else {
                return deny("not_ready", "Film nicht gefunden oder nicht bereit.");
            };
            room.set_stage(media, 0.0, false);
            broadcast_playlist(state, room).await;
        }
        ControlAction::Play => {
            if room.media.is_none() || room.playing {
                return Ok(());
            }
            room.base_at_ms = now_f();
            room.playing = true;
            persist_show_cursor(state, room).await;
        }
        ControlAction::Pause => {
            if room.media.is_none() || !room.playing {
                return Ok(());
            }
            room.base_pos_ms = room.current_pos_ms();
            room.playing = false;
            persist_show_cursor(state, room).await;
        }
        ControlAction::Seek => {
            let (Some(media), Some(pos)) = (&room.media, position_ms) else { return Ok(()) };
            let max = media.duration_ms.map(|d| d as f64).unwrap_or(f64::MAX);
            room.base_pos_ms = pos.clamp(0.0, max);
            room.base_at_ms = now_f();
            persist_show_cursor(state, room).await;
        }
        ControlAction::Stop => {
            if show_live {
                return deny("event_live", "Eine Vorstellung läuft – beende sie zuerst in der Konsole.");
            }
            room.clear_stage();
            broadcast_playlist(state, room).await;
        }
        ControlAction::Next => {
            if show_live {
                return deny("event_live", "Während einer Vorstellung gibt es kein Weiterschalten.");
            }
            advance_playlist(state, room).await;
            return Ok(());
        }
        ControlAction::PlayItem => {
            if show_live {
                return deny("event_live", "Eine Vorstellung läuft – beende sie zuerst in der Konsole.");
            }
            let Some(id) = item_id else { return Ok(()) };
            // The current item counts as played when jumping away.
            if let Some(cur) = room.current_item().filter(|c| *c != id) {
                let _ = watch::remove_playlist_item(&state.db, room.id(), cur).await;
            }
            let rows = watch::playlist(&state.db, room.id()).await.unwrap_or_default();
            let Some(row) = rows.into_iter().find(|r| r.id == id) else {
                return deny("not_found", "Eintrag nicht gefunden.");
            };
            if !start_playlist_row(state, room, row).await {
                advance_playlist(state, room).await;
            }
            broadcast_playlist(state, room).await;
        }
        ControlAction::StartEvent => return handle_start_event(state, room, event_id).await,
        ControlAction::SkipItem => {
            let index = match &room.show {
                Some(s) if matches!(s.phase, Phase::Item) => s.index,
                _ => return Ok(()),
            };
            advance_past_item(state, room, index).await;
            return Ok(());
        }
        ControlAction::EndIntermission => {
            let index = match &room.show {
                Some(s) if matches!(s.phase, Phase::Intermission { .. }) => s.index,
                _ => return Ok(()),
            };
            start_show_item(state, room, index + 1, 0.0, false).await;
            return Ok(());
        }
        ControlAction::ExtendIntermission => {
            let Some(delta) = delta_ms else { return Ok(()) };
            let Some(s) = room.show.as_mut() else { return Ok(()) };
            let Phase::Intermission { ends_at_ms } = s.phase else { return Ok(()) };
            let delta = delta.clamp(-MAX_INTERMISSION_SHIFT_MS, MAX_INTERMISSION_SHIFT_MS);
            let ends = (now_ms() + 5000).max(ends_at_ms + delta);
            s.phase = Phase::Intermission { ends_at_ms: ends };
            let (event_id, index) = (s.event_id, s.index);
            persist_cursor(
                &state.db,
                event_id,
                Cursor { item_index: index, phase: CursorPhase::Intermission, base_pos_ms: 0, base_at_ms: ends },
                false,
            )
            .await;
            after_show_change(state, room);
            return Ok(());
        }
        ControlAction::EndEvent => {
            let Some(live_now) = room.show.as_ref().map(|s| s.live) else { return Ok(()) };
            end_show(state, room, if live_now { EventStatus::Ended } else { EventStatus::Cancelled }).await;
            return Ok(());
        }
        ControlAction::SetLeader => {
            if !controller {
                return deny("forbidden", "Nur Moderatoren vergeben den Leader.");
            }
            if let Some(t) = target_user {
                if !state.hub.user_in_topic(t, &Topic::Watch(room.id())) {
                    return deny("not_found", "Dieser Nutzer ist nicht im Raum.");
                }
            }
            let before = room.leader;
            room.leader = target_user;
            for u in before.into_iter().chain(target_user) {
                send_you(state, room, u).await;
            }
            broadcast_presence(state, room).await;
            return Ok(());
        }
    }
    broadcast_state(state, room);
    Ok(())
}

async fn handle_voteskip(state: &AppState, room: &mut LiveRoom, user: &AuthUser) -> Reply {
    if room.show.as_ref().is_some_and(|s| s.live) {
        return deny("event_live", "Während einer Vorstellung kann nicht übersprungen werden.");
    }
    if room.media.is_none() {
        return Ok(());
    }
    if !room.votes.insert(user.id) {
        room.votes.remove(&user.id);
    }
    let (votes, needed) = voteskip_numbers(state, room);
    if votes >= needed {
        advance_playlist(state, room).await;
    } else {
        broadcast_voteskip(state, room);
    }
    Ok(())
}

async fn handle_duration(state: &AppState, room: &mut LiveRoom, key: &str, duration_ms: i64) -> Reply {
    if !(1000..=MAX_DURATION_MS).contains(&duration_ms) {
        return Ok(());
    }
    let Some(media) = room.media.as_mut() else { return Ok(()) };
    if media.key != key || media.duration_ms.is_some() {
        return Ok(());
    }
    media.duration_ms = Some(duration_ms);
    if let Some(item) = media.item_id {
        let _ = watch::set_item_duration(&state.db, item, duration_ms).await;
    }
    broadcast_state(state, room);
    broadcast_playlist(state, room).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------

fn playing_from_row(row: &PlaylistRow) -> Option<PlayingMedia> {
    let (kind, url) = match row.kind {
        SourceKind::Youtube => (PlayKind::Youtube, row.url.clone()?),
        SourceKind::Url => {
            let url = row.url.clone()?;
            let path = url.split(['?', '#']).next().unwrap_or("").to_lowercase();
            (if path.ends_with(".m3u8") { PlayKind::Hls } else { PlayKind::Direct }, url)
        }
        SourceKind::File => (PlayKind::Direct, format!("/files/{}", row.file_id?)),
        SourceKind::Library => return None,
    };
    Some(PlayingMedia {
        key: format!("item:{}", row.id),
        item_id: Some(row.id),
        media_id: None,
        title: row.title.clone(),
        kind,
        url,
        duration_ms: row.duration_ms,
        tracks: Vec::new(),
        poster_url: row.thumb_url.clone(),
        subs_base: None,
    })
}

/// Puts a playlist row on stage. False if it cannot be played.
async fn start_playlist_row(state: &AppState, room: &mut LiveRoom, row: PlaylistRow) -> bool {
    let media = match row.kind {
        SourceKind::Library => {
            let Some(id) = row.media_id else { return false };
            match crate::watch_media::playing_media(&state.db, id).await.ok().flatten() {
                Some(mut m) => {
                    m.key = format!("item:{}", row.id);
                    m.item_id = Some(row.id);
                    m
                }
                None => return false,
            }
        }
        _ => match playing_from_row(&row) {
            Some(m) => m,
            None => return false,
        },
    };
    room.set_stage(media, 0.0, false);
    true
}

/// The current item is done: drop it and start the next one (or go idle).
async fn advance_playlist(state: &AppState, room: &mut LiveRoom) {
    if let Some(cur) = room.current_item() {
        let _ = watch::remove_playlist_item(&state.db, room.id(), cur).await;
    }
    room.clear_stage();
    let rows = watch::playlist(&state.db, room.id()).await.unwrap_or_default();
    for row in rows {
        let id = row.id;
        if start_playlist_row(state, room, row).await {
            break;
        }
        // Unplayable (e.g. library item no longer ready): skip it.
        let _ = watch::remove_playlist_item(&state.db, room.id(), id).await;
    }
    broadcast_state(state, room);
    broadcast_playlist(state, room).await;
    broadcast_voteskip(state, room);
}

/// Starts the queue when the room is idle (after adds or a finished show).
async fn autoplay(state: &AppState, room: &mut LiveRoom) {
    if room.media.is_none() && room.show.is_none() {
        advance_playlist(state, room).await;
    }
}

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(StdDuration::from_secs(6))
            .user_agent("woelki-community/1.0")
            .build()
            .expect("HTTP client")
    })
}

async fn youtube_title(id: &str) -> Option<String> {
    let url = format!("https://www.youtube.com/oembed?url=https://www.youtube.com/watch?v={id}&format=json");
    let res = http().get(url).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let v: Value = res.json().await.ok()?;
    v.get("title").and_then(|t| t.as_str()).map(|t| t.chars().take(200).collect())
}

fn title_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("Video");
    let decoded = last.replace("%20", " ").replace('+', " ");
    let stem = decoded.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(decoded);
    if stem.is_empty() {
        "Video".into()
    } else {
        stem.chars().take(200).collect()
    }
}

async fn resolve_source(state: &AppState, user: &AuthUser, source: PlaylistSource) -> Result<watch::NewItem> {
    match source {
        PlaylistSource::Url { url } => {
            let url = url.trim().to_string();
            if let Some(id) = shared::watch::youtube_id(&url) {
                let title = youtube_title(&id).await.unwrap_or_else(|| format!("YouTube-Video {id}"));
                return Ok(watch::NewItem {
                    kind: SourceKind::Youtube,
                    media_id: None,
                    file_id: None,
                    thumb_url: Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")),
                    url: Some(id),
                    title,
                    duration_ms: None,
                });
            }
            if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 1000 {
                return Err(Error::invalid("Bitte gib einen YouTube-Link oder eine direkte Video-URL an."));
            }
            let path = url.split(['?', '#']).next().unwrap_or("").to_lowercase();
            const EXT: &[&str] = &[".mp4", ".webm", ".m4v", ".mov", ".ogv", ".ogg", ".m3u8"];
            if !EXT.iter().any(|e| path.ends_with(e)) {
                return Err(Error::invalid(
                    "Unterstützt werden YouTube-Links und direkte Videodateien (mp4, webm, ogg, m3u8).",
                ));
            }
            Ok(watch::NewItem {
                kind: SourceKind::Url,
                media_id: None,
                file_id: None,
                title: title_from_url(&url),
                url: Some(url),
                duration_ms: None,
                thumb_url: None,
            })
        }
        PlaylistSource::Library { media_id } => {
            let media = crate::watch_media::playing_media(&state.db, media_id)
                .await?
                .ok_or_else(|| Error::invalid("Dieser Film ist nicht bereit."))?;
            Ok(watch::NewItem {
                kind: SourceKind::Library,
                media_id: Some(media_id),
                file_id: None,
                url: None,
                title: media.title,
                duration_ms: media.duration_ms,
                thumb_url: media.poster_url,
            })
        }
        PlaylistSource::File { file_id } => {
            let rec = crate::files::get_record(&state.db, file_id).await?.ok_or(Error::NotFound("Datei"))?;
            if !crate::files::can_view(state, &rec, Some(user)).await || rec.view.deleted_at.is_some() {
                return Err(Error::NotFound("Datei"));
            }
            if !rec.view.mime_type.starts_with("video/") {
                return Err(Error::invalid("Bitte wähle eine Videodatei."));
            }
            if rec.view.visibility == shared::files::Visibility::Private {
                return Err(Error::invalid(
                    "Die Datei ist privat – stelle sie in „Dateien“ mindestens auf „Mitglieder“, damit alle sie sehen können.",
                ));
            }
            Ok(watch::NewItem {
                kind: SourceKind::File,
                media_id: None,
                file_id: Some(file_id),
                url: None,
                title: rec.view.name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(rec.view.name.clone()),
                duration_ms: rec.view.duration_secs.map(|s| (s * 1000.0) as i64),
                thumb_url: rec.view.has_thumb.then(|| format!("/files/{file_id}/thumb")),
            })
        }
    }
}

pub async fn add_to_playlist(state: &AppState, user: &AuthUser, room_id: Uuid, source: PlaylistSource, next: bool) -> Result<()> {
    let (row, my_role) = watch::accessible(&state.db, user, room_id).await?;
    let room_arc = get_room(state, room_id).await.ok_or(Error::NotFound("Raum"))?;
    {
        let room = room_arc.lock().await;
        let leader = room.leader == Some(user.id);
        if !watch::can_add(&row, user, my_role, leader) {
            return Err(Error::invalid("In diesem Raum dürfen nur Moderatoren die Playlist füllen."));
        }
        if next && !watch::can_control(user, my_role) && !leader {
            return Err(Error::Forbidden);
        }
    }
    state
        .limiter
        .hit(&format!("watchadd:{}", user.id), 20, StdDuration::from_secs(60))?;
    // Resolve outside the room lock (may call YouTube).
    let item = resolve_source(state, user, source).await?;
    watch::insert_playlist_item(&state.db, room_id, user, item, next).await?;
    let mut room = room_arc.lock().await;
    broadcast_playlist(state, &room).await;
    autoplay(state, &mut room).await;
    Ok(())
}

pub async fn remove_from_playlist(state: &AppState, user: &AuthUser, room_id: Uuid, item_id: Uuid) -> Result<()> {
    let (_, my_role) = watch::accessible(&state.db, user, room_id).await?;
    let added_by: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT added_by FROM watch_playlist WHERE id = $1 AND room_id = $2")
            .bind(item_id)
            .bind(room_id)
            .fetch_optional(&state.db)
            .await?;
    let added_by = added_by.ok_or(Error::NotFound("Eintrag"))?;
    let room_arc = get_room(state, room_id).await.ok_or(Error::NotFound("Raum"))?;
    let mut room = room_arc.lock().await;
    let allowed = watch::can_control(user, my_role) || room.leader == Some(user.id) || added_by == Some(user.id);
    if !allowed {
        return Err(Error::Forbidden);
    }
    if room.current_item() == Some(item_id) {
        if room.show.as_ref().is_some_and(|s| s.live) {
            return Err(Error::invalid("Eine Vorstellung läuft."));
        }
        advance_playlist(state, &mut room).await;
    } else {
        watch::remove_playlist_item(&state.db, room_id, item_id).await?;
        broadcast_playlist(state, &room).await;
    }
    Ok(())
}

async fn playlist_admin(state: &AppState, user: &AuthUser, room_id: Uuid) -> Result<Arc<Mutex<LiveRoom>>> {
    let (_, my_role) = watch::accessible(&state.db, user, room_id).await?;
    let room_arc = get_room(state, room_id).await.ok_or(Error::NotFound("Raum"))?;
    let leader = room_arc.lock().await.leader == Some(user.id);
    if !watch::can_control(user, my_role) && !leader {
        return Err(Error::Forbidden);
    }
    Ok(room_arc)
}

pub async fn move_in_playlist(state: &AppState, user: &AuthUser, room_id: Uuid, item_id: Uuid, before: Option<Uuid>) -> Result<()> {
    let room_arc = playlist_admin(state, user, room_id).await?;
    watch::move_playlist_item(&state.db, room_id, item_id, before).await?;
    let room = room_arc.lock().await;
    // Keep the playing item on top.
    if let Some(cur) = room.current_item().filter(|c| *c != item_id) {
        let first = watch::playlist(&state.db, room_id).await?.first().map(|r| r.id);
        if first != Some(cur) {
            watch::move_playlist_item(&state.db, room_id, cur, first).await?;
        }
    }
    broadcast_playlist(state, &room).await;
    Ok(())
}

pub async fn shuffle_playlist(state: &AppState, user: &AuthUser, room_id: Uuid) -> Result<()> {
    let room_arc = playlist_admin(state, user, room_id).await?;
    watch::shuffle_playlist(&state.db, room_id).await?;
    let room = room_arc.lock().await;
    if let Some(cur) = room.current_item() {
        let first = watch::playlist(&state.db, room_id).await?.first().map(|r| r.id);
        watch::move_playlist_item(&state.db, room_id, cur, first).await?;
    }
    broadcast_playlist(state, &room).await;
    Ok(())
}

pub async fn clear_playlist(state: &AppState, user: &AuthUser, room_id: Uuid) -> Result<()> {
    let room_arc = playlist_admin(state, user, room_id).await?;
    let room = room_arc.lock().await;
    watch::clear_playlist(&state.db, room_id, room.current_item()).await?;
    broadcast_playlist(state, &room).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scheduled show engine (HQTube)
// ---------------------------------------------------------------------------

async fn persist_cursor(db: &sqlx::PgPool, event_id: Uuid, c: Cursor, set_live: bool) {
    let phase = match c.phase {
        CursorPhase::Preshow => "preshow",
        CursorPhase::Playing => "playing",
        CursorPhase::Paused => "paused",
        CursorPhase::Intermission => "intermission",
    };
    let sql = if set_live {
        "UPDATE watch_events SET item_index = $2, phase = $3, base_pos_ms = $4, base_at_ms = $5, status = 'live' WHERE id = $1"
    } else {
        "UPDATE watch_events SET item_index = $2, phase = $3, base_pos_ms = $4, base_at_ms = $5 WHERE id = $1"
    };
    if let Err(e) = sqlx::query(sql)
        .bind(event_id)
        .bind(c.item_index as i32)
        .bind(phase)
        .bind(c.base_pos_ms)
        .bind(c.base_at_ms)
        .execute(db)
        .await
    {
        tracing::warn!(error = %e, "persisting show cursor failed");
    }
}

/// Terminal transition — clears the cursor so a restart never resumes it.
async fn finish_event(db: &sqlx::PgPool, event_id: Uuid, status: EventStatus) {
    let _ = sqlx::query(
        "UPDATE watch_events SET status = $2, item_index = NULL, phase = NULL, base_pos_ms = NULL, base_at_ms = NULL WHERE id = $1",
    )
    .bind(event_id)
    .bind(status.as_str())
    .execute(db)
    .await;
}

struct EventRow {
    room_id: Uuid,
    title: String,
    starts_at: i64,
    status: EventStatus,
    cursor: Option<Cursor>,
}

async fn load_event(db: &sqlx::PgPool, id: Uuid) -> Option<(EventRow, Vec<ShowItem>)> {
    let r = sqlx::query(
        "SELECT room_id, title, starts_at, status, item_index, phase, base_pos_ms, base_at_ms FROM watch_events WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
    .ok()??;
    let phase: Option<String> = r.try_get("phase").ok()?;
    let item_index: Option<i32> = r.try_get("item_index").ok()?;
    let base_pos: Option<i64> = r.try_get("base_pos_ms").ok()?;
    let base_at: Option<i64> = r.try_get("base_at_ms").ok()?;
    let cursor = match (item_index, phase.as_deref(), base_pos, base_at) {
        (Some(i), Some(p), Some(pos), Some(at)) => Some(Cursor {
            item_index: i as i64,
            phase: match p {
                "playing" => CursorPhase::Playing,
                "paused" => CursorPhase::Paused,
                "intermission" => CursorPhase::Intermission,
                _ => CursorPhase::Preshow,
            },
            base_pos_ms: pos,
            base_at_ms: at,
        }),
        _ => None,
    };
    let items = watch::event_items(db, &[id]).await.ok()?.into_iter().map(|(_, i)| i).collect();
    Some((
        EventRow {
            room_id: r.try_get("room_id").ok()?,
            title: r.try_get("title").ok()?,
            starts_at: r.try_get("starts_at").ok()?,
            status: EventStatus::parse(&r.try_get::<String, _>("status").ok()?),
            cursor,
        },
        items,
    ))
}

fn engine_items(items: &[ShowItem]) -> Vec<EngineItem> {
    items
        .iter()
        .map(|i| EngineItem { duration_ms: i.duration_ms, intermission_after_ms: i.intermission_after_ms })
        .collect()
}

/// Called by the scheduler for a due event. Idempotent; recovers mid-show
/// positions after a restart.
async fn adopt_show(state: &AppState, room: &mut LiveRoom, event_id: Uuid) {
    if room.show.is_some() || room.adopting {
        return;
    }
    room.adopting = true;
    let Some((ev, items)) = load_event(&state.db, event_id).await else {
        room.adopting = false;
        return;
    };
    room.adopting = false;
    if !matches!(ev.status, EventStatus::Scheduled | EventStatus::Live) {
        return;
    }
    let now = now_ms();
    let show = LiveShow {
        event_id,
        title: ev.title.clone(),
        starts_at_ms: ev.starts_at,
        live: ev.status == EventStatus::Live,
        items: items.clone(),
        index: -1,
        phase: Phase::Preshow,
    };
    if ev.status == EventStatus::Scheduled && now < ev.starts_at {
        room.show = Some(show);
        after_show_change(state, room);
        return;
    }
    let engine = engine_items(&items);
    let recovery = match (ev.status, ev.cursor) {
        (EventStatus::Live, Some(c)) => watch_engine::resume_from_cursor(c, &engine, now),
        _ => watch_engine::recover_from_schedule(ev.starts_at, &engine, now),
    };
    apply_recovery(state, room, show, recovery).await;
}

async fn apply_recovery(state: &AppState, room: &mut LiveRoom, mut show: LiveShow, recovery: Recovery) {
    if recovery == Recovery::Ended {
        // The whole runtime elapsed while the server was down.
        finish_event(&state.db, show.event_id, EventStatus::Ended).await;
        return;
    }
    show.live = true;
    match recovery {
        Recovery::Intermission { item_index, ends_at_ms } => {
            show.index = item_index as i64;
            show.phase = Phase::Intermission { ends_at_ms };
            let event_id = show.event_id;
            room.clear_stage();
            room.show = Some(show);
            persist_cursor(
                &state.db,
                event_id,
                Cursor {
                    item_index: item_index as i64,
                    phase: CursorPhase::Intermission,
                    base_pos_ms: 0,
                    base_at_ms: ends_at_ms,
                },
                true,
            )
            .await;
            after_show_change(state, room);
        }
        Recovery::Playing { item_index, position_ms } | Recovery::Paused { item_index, position_ms } => {
            let paused = matches!(recovery, Recovery::Paused { .. });
            room.show = Some(show);
            start_show_item(state, room, item_index as i64, position_ms as f64, paused).await;
        }
        Recovery::Ended => {}
    }
}

/// Loads and starts a show item, auto-skipping anything unplayable.
async fn start_show_item(state: &AppState, room: &mut LiveRoom, index: i64, pos_ms: f64, paused: bool) {
    let Some(show) = room.show.clone() else { return };
    let mut i = index.max(0) as usize;
    while i < show.items.len() {
        let item = &show.items[i];
        if let Some(mut media) = crate::watch_media::playing_media(&state.db, item.media_id).await.ok().flatten() {
            media.key = format!("show:{}:{i}", show.event_id);
            let start = if i as i64 == index {
                pos_ms.clamp(0.0, media.duration_ms.map(|d| d as f64).unwrap_or(pos_ms.max(0.0)))
            } else {
                0.0
            };
            room.set_stage(media, start, paused);
            if let Some(s) = room.show.as_mut() {
                s.index = i as i64;
                s.phase = Phase::Item;
                s.live = true;
            }
            persist_cursor(
                &state.db,
                show.event_id,
                Cursor {
                    item_index: i as i64,
                    phase: if paused { CursorPhase::Paused } else { CursorPhase::Playing },
                    base_pos_ms: start as i64,
                    base_at_ms: room.base_at_ms as i64,
                },
                true,
            )
            .await;
            after_show_change(state, room);
            broadcast_playlist(state, room).await;
            return;
        }
        i += 1;
    }
    // Nothing playable left in the line-up.
    end_show(state, room, EventStatus::Ended).await;
}

/// An item finished (naturally or skipped): break, next item, or the end.
async fn advance_past_item(state: &AppState, room: &mut LiveRoom, index: i64) {
    let Some(show) = room.show.clone() else { return };
    room.clear_stage();
    if index >= show.items.len() as i64 - 1 {
        end_show(state, room, EventStatus::Ended).await;
        return;
    }
    let break_ms = show.items.get(index as usize).map(|i| i.intermission_after_ms).unwrap_or(0);
    if break_ms > 0 {
        let ends = now_ms() + break_ms;
        if let Some(s) = room.show.as_mut() {
            s.phase = Phase::Intermission { ends_at_ms: ends };
        }
        persist_cursor(
            &state.db,
            show.event_id,
            Cursor { item_index: index, phase: CursorPhase::Intermission, base_pos_ms: 0, base_at_ms: ends },
            false,
        )
        .await;
        after_show_change(state, room);
        return;
    }
    start_show_item(state, room, index + 1, 0.0, false).await;
}

async fn end_show(state: &AppState, room: &mut LiveRoom, status: EventStatus) {
    let Some(show) = room.show.take() else { return };
    room.clear_stage();
    if status == EventStatus::Ended && show.live {
        room.ended_card = Some((show.event_id, show.title.clone(), now_ms()));
    }
    finish_event(&state.db, show.event_id, status).await;
    after_show_change(state, room);
    // Ad-hoc queue continues after the show.
    if !watch::playlist(&state.db, room.id()).await.unwrap_or_default().is_empty() {
        room.ended_card = None;
        autoplay(state, room).await;
    } else {
        broadcast_playlist(state, room).await;
    }
    scheduler_kick(state);
}

async fn handle_start_event(state: &AppState, room: &mut LiveRoom, event_id: Option<Uuid>) -> Reply {
    let Some(event_id) = event_id else { return Ok(()) };
    if room.show.as_ref().is_some_and(|s| s.event_id != event_id) {
        return deny("event_live", "In diesem Raum läuft schon eine andere Vorstellung.");
    }
    if room.show.as_ref().is_some_and(|s| s.live) {
        return Ok(());
    }
    if room.adopting {
        return deny("busy", "Gleich nochmal versuchen.");
    }
    let now = now_ms();
    if room.show.is_none() {
        // Beyond the preshow lead the room has not adopted it yet.
        let Some((ev, items)) = load_event(&state.db, event_id).await else {
            return deny("bad_state", "Vorstellung nicht gefunden.");
        };
        if ev.room_id != room.id() || ev.status != EventStatus::Scheduled {
            return deny("bad_state", "Diese Vorstellung kann nicht gestartet werden.");
        }
        room.show = Some(LiveShow {
            event_id,
            title: ev.title,
            starts_at_ms: now,
            live: false,
            items,
            index: -1,
            phase: Phase::Preshow,
        });
    }
    if let Some(s) = room.show.as_mut() {
        s.starts_at_ms = now;
    }
    // "Start now" moves the schedule so recovery math stays truthful.
    let _ = sqlx::query("UPDATE watch_events SET starts_at = $2 WHERE id = $1")
        .bind(event_id)
        .bind(now)
        .execute(&state.db)
        .await;
    start_show_item(state, room, 0, 0.0, false).await;
    Ok(())
}

async fn persist_show_cursor(state: &AppState, room: &LiveRoom) {
    let Some(s) = &room.show else { return };
    if !matches!(s.phase, Phase::Item) {
        return;
    }
    persist_cursor(
        &state.db,
        s.event_id,
        Cursor {
            item_index: s.index,
            phase: if room.playing { CursorPhase::Playing } else { CursorPhase::Paused },
            base_pos_ms: room.base_pos_ms as i64,
            base_at_ms: room.base_at_ms as i64,
        },
        false,
    )
    .await;
}

/// Re-reads an adopted-but-not-live event after schedule edits.
pub async fn refresh_show(state: &AppState, room_id: Uuid, event_id: Uuid) {
    let Some(room_arc) = loaded(state, room_id) else {
        scheduler_kick(state);
        return;
    };
    let mut room = room_arc.lock().await;
    let Some(s) = room.show.as_ref() else {
        scheduler_kick(state);
        return;
    };
    if s.event_id != event_id || s.live {
        return;
    }
    match load_event(&state.db, event_id).await {
        Some((ev, items)) if ev.status == EventStatus::Scheduled && ev.starts_at - now_ms() <= PRESHOW_LEAD_MS => {
            if let Some(s) = room.show.as_mut() {
                s.title = ev.title;
                s.starts_at_ms = ev.starts_at;
                s.items = items;
            }
        }
        _ => room.show = None,
    }
    after_show_change(state, &room);
    scheduler_kick(state);
}

// ---------------------------------------------------------------------------
// Ticker + scheduler
// ---------------------------------------------------------------------------

async fn tick(state: &AppState, room: &mut LiveRoom) -> bool {
    room.tick_count += 1;
    let now = now_ms();
    let viewers = state.hub.topic_conn_count(&Topic::Watch(room.id()));
    if let Some(s) = room.show.clone() {
        match s.phase {
            Phase::Preshow => {
                // Seizes control even if ad-hoc playback is still running.
                if now >= s.starts_at_ms {
                    start_show_item(state, room, 0, 0.0, false).await;
                    return true;
                }
            }
            Phase::Item => {
                let ended = match &room.media {
                    None => true,
                    Some(m) => room.playing && m.duration_ms.is_some_and(|d| room.raw_pos_ms() >= d as f64 + END_SLACK_MS - 1.0),
                };
                if ended {
                    advance_past_item(state, room, s.index).await;
                    return true;
                }
            }
            Phase::Intermission { ends_at_ms } => {
                if now >= ends_at_ms {
                    start_show_item(state, room, s.index + 1, 0.0, false).await;
                    return true;
                }
            }
        }
    } else if let Some(m) = &room.media {
        if room.playing && m.duration_ms.is_some_and(|d| room.raw_pos_ms() >= d as f64 + END_SLACK_MS - 1.0) {
            if m.item_id.is_some() {
                advance_playlist(state, room).await;
            } else {
                room.clear_stage();
                broadcast_state(state, room);
                autoplay(state, room).await;
            }
            return true;
        }
    } else if let Some((_, _, at)) = room.ended_card {
        if now - at >= ENDED_CARD_MS {
            room.ended_card = None;
            broadcast_state(state, room);
        }
    }
    // Periodic re-broadcast for drift correction.
    if room.media.is_some() && room.tick_count % STATE_REBROADCAST_TICKS == 0 && viewers > 0 {
        broadcast_state(state, room);
    }
    // Empty rooms without a show are released.
    if viewers == 0 && room.show.is_none() {
        let since = *room.empty_since.get_or_insert(now);
        if now - since > DISPOSE_AFTER_MS {
            return false;
        }
    } else {
        room.empty_since = None;
    }
    true
}

async fn tick_all(state: &AppState) {
    let rooms: Vec<(Uuid, Arc<Mutex<LiveRoom>>)> =
        live(state).rooms.lock().expect("rooms").iter().map(|(k, v)| (*k, v.clone())).collect();
    for (id, room_arc) in rooms {
        let keep = {
            let mut room = room_arc.lock().await;
            tick(state, &mut room).await
        };
        if !keep {
            live(state).rooms.lock().expect("rooms").remove(&id);
            live(state).summaries.lock().expect("summaries").remove(&id);
        }
    }
}

async fn scheduler_tick(state: &AppState) {
    let due: Vec<(Uuid, Uuid)> = match sqlx::query_as(
        "SELECT id, room_id FROM watch_events WHERE status = 'live' OR (status = 'scheduled' AND starts_at <= $1) \
         ORDER BY starts_at",
    )
    .bind(now_ms() + PRESHOW_LEAD_MS)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "watch scheduler failed");
            return;
        }
    };
    let mut seen = HashSet::new();
    for (event_id, room_id) in due {
        // The earliest event per room wins; later ones wait.
        if !seen.insert(room_id) {
            continue;
        }
        if let Some(room_arc) = get_room(state, room_id).await {
            let mut room = room_arc.lock().await;
            room.empty_since = None;
            adopt_show(state, &mut room, event_id).await;
        }
    }
}

/// Starts the ticker and the show scheduler.
pub fn start(state: AppState) {
    let ticker_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(StdDuration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            tick_all(&ticker_state).await;
        }
    });
    tokio::spawn(async move {
        loop {
            scheduler_tick(&state).await;
            tokio::select! {
                _ = tokio::time::sleep(StdDuration::from_secs(5)) => {}
                _ = live(&state).scheduler.notified() => {}
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Hooks for REST-side changes
// ---------------------------------------------------------------------------

pub async fn apply_moderation(state: &AppState, room_id: Uuid, user_id: Uuid, effect: ModEffect) {
    let topic = Topic::Watch(room_id);
    if effect.banned {
        for conn in state.hub.user_conns_in_topic(user_id, &topic) {
            state.hub.send_conn(
                conn,
                &emit(&WatchEvent::Kicked { room_id, reason: "Du wurdest aus diesem Raum verbannt.".into() }),
            );
            state.hub.unsubscribe(conn, &topic);
        }
        on_leave(state, room_id).await;
        return;
    }
    let Some(room_arc) = loaded(state, room_id) else { return };
    let mut room = room_arc.lock().await;
    match effect.muted_until_ms {
        Some(t) => room.muted_until.insert(user_id, t),
        None => room.muted_until.remove(&user_id),
    };
    send_you(state, &mut room, user_id).await;
}

pub async fn refresh_user(state: &AppState, room_id: Uuid, user_id: Uuid) {
    let Some(room_arc) = loaded(state, room_id) else { return };
    let mut room = room_arc.lock().await;
    room.roles.remove(&user_id);
    send_you(state, &mut room, user_id).await;
    broadcast_presence(state, &mut room).await;
}

pub async fn apply_slow_mode(state: &AppState, room_id: Uuid, ms: i64) {
    if let Some(room_arc) = loaded(state, room_id) {
        room_arc.lock().await.row.slow_mode_ms = ms;
    }
    publish(state, room_id, &WatchEvent::SlowMode { room_id, ms });
}

pub fn chat_removed(state: &AppState, room_id: Uuid, ids: Vec<Uuid>) {
    publish(state, room_id, &WatchEvent::ChatRemoved { room_id, ids });
}

pub fn proposals_changed(state: &AppState, room_id: Uuid) {
    publish(state, room_id, &WatchEvent::ProposalsChanged { room_id });
}

pub async fn room_settings_changed(state: &AppState, room_id: Uuid) {
    if let Some(room_arc) = loaded(state, room_id) {
        if let Ok(Some(row)) = watch::room_row(&state.db, room_id).await {
            let mut room = room_arc.lock().await;
            room.row = row;
            room.roles.clear();
        }
    }
    publish(state, room_id, &WatchEvent::RoomUpdated { room_id });
}

pub async fn close_room(state: &AppState, room_id: Uuid, reason: &str) {
    publish(state, room_id, &WatchEvent::Kicked { room_id, reason: reason.into() });
    live(state).rooms.lock().expect("rooms").remove(&room_id);
    live(state).summaries.lock().expect("summaries").remove(&room_id);
}
