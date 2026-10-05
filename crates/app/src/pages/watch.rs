//! Watch together ("Kino"): room directory, room page with the synced
//! stage, chat with emotes, playlist, requests, viewers, and popouts. The
//! showmaster console lives in `watch_console`, the library in
//! `watch_library`. The player itself is `assets/js/wcf-watch.js`.

use std::collections::HashMap;

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_params_map};
use shared::watch::{
    fmt_timecode, ControlAction, Emote, ModerationKind, PlaybackState, PlaylistItem, PlaylistMode, PlaylistSource,
    PresenceUser, ProposalInput, ProposalStatus, RoomRole, ShowInfo, WatchChatMessage, WatchClientMsg,
    WatchEvent, WatchRoomInput, WatchRoomView, WatchVisibility, WatchYou, CHAT_MAX_CHARS, MODULE,
};
use shared::ws::{ClientMsg, ServerMsg, Topic};
use shared::UserSummary;
use uuid::Uuid;

use crate::api::watch::*;
use crate::components::common::{confirm, copy_to_clipboard, local_time, origin};
use crate::components::user_picker::UserPicker;
use crate::components::{Avatar, EmptyState, ErrorBox, Icon, Modal, RequireLogin, Spinner};
use crate::pages::not_found::Missing;
use crate::realtime::{use_realtime, Realtime};
use crate::state::{now_ms, use_session, use_toasts};

// ---------------------------------------------------------------------------
// JS bridge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PlayerEvent {
    Duration {
        key: String,
        ms: i64,
    },
    /// Audio election changes (the player shows its own hints).
    Audio {},
    Error {
        message: String,
    },
}

fn on_player_event(handler: impl Fn(PlayerEvent) + 'static) {
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::JsCast;
        let h = window_event_listener_untyped("wcf-watch", move |ev| {
            if let Some(ce) = ev.dyn_ref::<web_sys::CustomEvent>() {
                if let Some(s) = ce.detail().as_string() {
                    if let Ok(e) = serde_json::from_str::<PlayerEvent>(&s) {
                        handler(e);
                    }
                }
            }
        });
        on_cleanup(move || h.remove());
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = handler;
}

/// A signal with the current wall clock, ticking every `ms` on the client.
pub(crate) fn ticker(ms: u64) -> RwSignal<f64> {
    let now = RwSignal::new(now_ms());
    #[cfg(feature = "hydrate")]
    {
        let handle = set_interval_with_handle(move || now.set(now_ms()), std::time::Duration::from_millis(ms)).ok();
        on_cleanup(move || {
            if let Some(h) = handle {
                h.clear();
            }
        });
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = ms;
    now
}

/// Reads a `LocalResource` only in the browser. Server rendering (and the
/// first client render) see `None`, which keeps hydration consistent and
/// avoids reading local resources outside of a Suspense on the server.
pub(crate) fn local<T: Clone + Send + Sync + 'static>(r: LocalResource<T>) -> Option<T> {
    #[cfg(feature = "hydrate")]
    {
        r.get()
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = r;
        None
    }
}

pub(crate) fn clock_of(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(local_time).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Room connection (state mirrored from the server)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct RoomConn {
    pub room_id: Uuid,
    rt: Realtime,
    pub room: RwSignal<Option<WatchRoomView>>,
    pub you: RwSignal<Option<WatchYou>>,
    pub state: RwSignal<PlaybackState>,
    pub show: RwSignal<Option<ShowInfo>>,
    pub playlist: RwSignal<Vec<PlaylistItem>>,
    pub current_item: RwSignal<Option<Uuid>>,
    pub chat: RwSignal<Vec<WatchChatMessage>>,
    pub presence: RwSignal<Vec<PresenceUser>>,
    pub slow_mode: RwSignal<i64>,
    pub voteskip: RwSignal<(usize, usize)>,
    pub voted: RwSignal<bool>,
    pub proposals_version: RwSignal<u32>,
    pub last_error: RwSignal<Option<(String, String, f64)>>,
    pub kicked: RwSignal<Option<String>>,
    pub welcomed: RwSignal<bool>,
    pub room_version: RwSignal<u32>,
}

impl RoomConn {
    pub fn new(room: WatchRoomView) -> Self {
        let rt = use_realtime();
        let c = RoomConn {
            room_id: room.id,
            rt,
            slow_mode: RwSignal::new(room.slow_mode_ms),
            room: RwSignal::new(Some(room)),
            you: RwSignal::new(None),
            state: RwSignal::new(PlaybackState::Idle),
            show: RwSignal::new(None),
            playlist: RwSignal::new(Vec::new()),
            current_item: RwSignal::new(None),
            chat: RwSignal::new(Vec::new()),
            presence: RwSignal::new(Vec::new()),
            voteskip: RwSignal::new((0, 1)),
            voted: RwSignal::new(false),
            proposals_version: RwSignal::new(0),
            last_error: RwSignal::new(None),
            kicked: RwSignal::new(None),
            welcomed: RwSignal::new(false),
            room_version: RwSignal::new(0),
        };
        rt.subscribe_scoped(Topic::Watch(c.room_id));
        rt.sync_clock();
        rt.on_message(move |msg| {
            if let ServerMsg::Module { module, payload } = msg {
                if let Some(ev) = WatchEvent::from_payload(module, payload) {
                    c.apply(ev);
                }
            }
        });
        c
    }

    fn apply(&self, ev: WatchEvent) {
        let me = self.room_id;
        match ev {
            WatchEvent::Welcome { room, you, state, show, playlist, current_item, chat, presence, voteskip } => {
                if room.id != me {
                    return;
                }
                self.slow_mode.set(room.slow_mode_ms);
                self.room.set(Some(room));
                self.you.set(Some(you));
                self.state.set(state);
                self.show.set(show);
                self.playlist.set(playlist);
                self.current_item.set(current_item);
                self.chat.set(chat);
                self.presence.set(presence);
                self.voteskip.set(voteskip);
                self.kicked.set(None);
                self.welcomed.set(true);
            }
            WatchEvent::State { room_id, state } if room_id == me => {
                let old_key = self.state.with_untracked(|s| s.media().map(|m| m.key.clone()));
                if old_key != state.media().map(|m| m.key.clone()) {
                    self.voted.set(false);
                }
                self.state.set(state);
            }
            WatchEvent::Show { room_id, show } if room_id == me => self.show.set(show),
            WatchEvent::Playlist { room_id, items, current_item } if room_id == me => {
                self.playlist.set(items);
                self.current_item.set(current_item);
            }
            WatchEvent::Chat { room_id, msg } if room_id == me => self.chat.update(|c| {
                c.push(msg);
                if c.len() > 300 {
                    c.drain(..c.len() - 300);
                }
            }),
            WatchEvent::ChatRemoved { room_id, ids } if room_id == me => {
                self.chat.update(|c| c.retain(|m| !ids.contains(&m.id)))
            }
            WatchEvent::Presence { room_id, users } if room_id == me => self.presence.set(users),
            WatchEvent::You { room_id, you } if room_id == me => self.you.set(Some(you)),
            WatchEvent::SlowMode { room_id, ms } if room_id == me => self.slow_mode.set(ms),
            WatchEvent::Voteskip { room_id, votes, needed } if room_id == me => {
                self.voteskip.set((votes, needed));
                if votes == 0 {
                    self.voted.set(false);
                }
            }
            WatchEvent::ProposalsChanged { room_id } if room_id == me => self.proposals_version.update(|v| *v += 1),
            WatchEvent::RoomUpdated { room_id } if room_id == me => self.room_version.update(|v| *v += 1),
            WatchEvent::Kicked { room_id, reason } if room_id == me => self.kicked.set(Some(reason)),
            WatchEvent::Error { room_id, code, message } if room_id == me => {
                self.last_error.set(Some((code, message, now_ms())))
            }
            _ => {}
        }
    }

    pub fn send(&self, msg: WatchClientMsg) {
        if let Ok(payload) = serde_json::to_value(&msg) {
            self.rt.send(&ClientMsg::Module { module: MODULE.into(), payload });
        }
    }

    pub fn control(&self, action: ControlAction, f: impl FnOnce(&mut CtlArgs)) {
        let mut a = CtlArgs::default();
        f(&mut a);
        self.send(WatchClientMsg::Control {
            room_id: self.room_id,
            action,
            position_ms: a.position_ms,
            media_id: a.media_id,
            item_id: a.item_id,
            event_id: a.event_id,
            delta_ms: a.delta_ms,
            user_id: a.user_id,
        });
    }

    pub fn can_control(&self) -> bool {
        self.you.with(|y| y.as_ref().is_some_and(|y| y.can_control))
    }

    pub fn is_leader(&self) -> bool {
        self.you.with(|y| y.as_ref().is_some_and(|y| y.leader))
    }

    pub fn can_drive(&self) -> bool {
        self.can_control() || self.is_leader()
    }

    pub fn show_live(&self) -> bool {
        self.show.with(|s| s.as_ref().is_some_and(|s| s.status == shared::watch::EventStatus::Live))
    }

    pub fn server_now(&self) -> f64 {
        now_ms() + self.rt.clock_offset.get_untracked()
    }

    pub fn offset(&self) -> RwSignal<f64> {
        self.rt.clock_offset
    }
}

#[derive(Default)]
pub struct CtlArgs {
    pub position_ms: Option<f64>,
    pub media_id: Option<Uuid>,
    pub item_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub delta_ms: Option<i64>,
    pub user_id: Option<Uuid>,
}

// ---------------------------------------------------------------------------
// Directory
// ---------------------------------------------------------------------------

#[component]
pub fn WatchIndex() -> impl IntoView {
    view! {
        <Title text="Kino"/>
        <RequireLogin>
            <WatchDirectory/>
        </RequireLogin>
    }
}

#[component]
fn WatchDirectory() -> impl IntoView {
    let session = use_session();
    let navigate = use_navigate();
    let version = RwSignal::new(0u32);
    let rooms = Resource::new(move || version.get(), |_| fetch_watch_rooms());
    #[cfg(feature = "hydrate")]
    {
        let handle = set_interval_with_handle(move || version.update(|v| *v += 1), std::time::Duration::from_secs(30)).ok();
        on_cleanup(move || {
            if let Some(h) = handle {
                h.clear();
            }
        });
    }
    let create_open = RwSignal::new(false);
    let go = Callback::new(move |p: String| navigate(&p, Default::default()));
    let on_created = Callback::new(move |r: WatchRoomView| {
        create_open.set(false);
        go.run(r.url());
    });
    let can_create = move || session.is_mod() || session.site.with(|s| s.members_create_watch_rooms);
    let tick = ticker(30_000);
    view! {
        <div class="page-header">
            <div>
                <h1>"Kino"</h1>
                <p class="muted">"Gemeinsam schauen: synchron, mit Chat, Playlist und geplanten Vorstellungen"</p>
            </div>
            <div class="page-actions">
                <Show when=move || session.is_admin()>
                    <a class="btn" href="/watch/library"><Icon name="film" size=16/>"Mediathek"</a>
                </Show>
                <Show when=can_create>
                    <button class="btn btn-primary" on:click=move |_| create_open.set(true)><Icon name="plus" size=16/>"Neuer Raum"</button>
                </Show>
            </div>
        </div>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match rooms.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(list) if list.is_empty() => view! {
                        <EmptyState icon="tv" title="Noch keine Kinoräume">
                            <p class="muted">"Lege den ersten Raum an und lade Freunde zum gemeinsamen Schauen ein."</p>
                        </EmptyState>
                    }.into_any(),
                    Ok(list) => view! {
                        <div class="watch-rooms">
                            {list.into_iter().map(|r| view! { <WatchRoomCard room=r tick=tick/> }).collect_view()}
                        </div>
                    }.into_any(),
                }
            })}
        </Transition>
        <Modal open=create_open title="Neuer Kinoraum">
            <WatchRoomForm room=None on_saved=on_created/>
        </Modal>
    }
}

#[component]
fn WatchRoomCard(room: WatchRoomView, tick: RwSignal<f64>) -> impl IntoView {
    let live = room.show.as_ref().is_some_and(|s| s.live);
    let next = room.next_event.clone();
    view! {
        <a class="watch-room-card card" class:live=live href=room.url()>
            <div class="row gap-sm">
                <strong class="grow ellipsis">{room.name.clone()}</strong>
                {live.then(|| view! { <span class="live-badge">"LIVE"</span> })}
                {(room.visibility == WatchVisibility::Private).then(|| view! { <span title="Privat"><Icon name="lock" size=14/></span> })}
            </div>
            {(!room.description.is_empty()).then(|| view! { <p class="muted small clamp-2">{room.description.clone()}</p> })}
            {match (&room.show, &room.now_playing) {
                (Some(s), _) if s.live => view! { <div class="watch-room-now"><Icon name="film" size=14/>{s.title.clone()}</div> }.into_any(),
                (_, Some(t)) => view! { <div class="watch-room-now"><Icon name="play" size=14/>{t.clone()}</div> }.into_any(),
                _ => ().into_any(),
            }}
            {next.map(|n| {
                let starts = n.starts_at_ms;
                view! {
                    <div class="watch-room-next muted small">
                        <Icon name="calendar" size=13/>
                        {n.title.clone()} " · "
                        {move || {
                            let _ = tick.get();
                            let mins = (starts as f64 - now_ms()) / 60_000.0;
                            if mins <= 0.0 { "jetzt".to_string() }
                            else if mins < 90.0 { format!("in {} Min.", mins.ceil() as i64) }
                            else { chrono::DateTime::from_timestamp_millis(starts).map(crate::components::common::local_datetime).unwrap_or_default() }
                        }}
                    </div>
                }
            })}
            <div class="row gap-sm muted small">
                <Icon name="users" size=13/>{format!("{} schauen", room.viewers)}
                <span class="grow"></span>
                <Avatar user=room.owner.clone() size=18/>{room.owner.display_name.clone()}
            </div>
        </a>
    }
}

#[component]
pub(crate) fn WatchRoomForm(room: Option<WatchRoomView>, on_saved: Callback<WatchRoomView>) -> impl IntoView {
    let toasts = use_toasts();
    let id = room.as_ref().map(|r| r.id);
    let name = RwSignal::new(room.as_ref().map(|r| r.name.clone()).unwrap_or_default());
    let description = RwSignal::new(room.as_ref().map(|r| r.description.clone()).unwrap_or_default());
    let visibility = RwSignal::new(room.as_ref().map(|r| r.visibility).unwrap_or_default());
    let mode = RwSignal::new(room.as_ref().map(|r| r.playlist_mode).unwrap_or_default());
    let voteskip = RwSignal::new(room.as_ref().map(|r| r.voteskip_percent).unwrap_or(50).to_string());
    let saving = RwSignal::new(false);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = WatchRoomInput {
            name: name.get_untracked(),
            description: description.get_untracked(),
            visibility: visibility.get_untracked(),
            playlist_mode: mode.get_untracked(),
            voteskip_percent: voteskip.get_untracked().trim().parse().unwrap_or(50),
        };
        saving.set(true);
        leptos::task::spawn_local(async move {
            let res = match id {
                Some(id) => update_watch_room(id, input).await,
                None => create_watch_room(input).await,
            };
            saving.set(false);
            match res {
                Ok(r) => on_saved.run(r),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <form class="form" on:submit=submit>
            <label class="field"><span>"Name"</span><input type="text" required maxlength="60" bind:value=name placeholder="z. B. Filmabend"/></label>
            <label class="field"><span>"Beschreibung"</span><textarea rows="2" maxlength="1000" bind:value=description></textarea></label>
            <div class="form-row">
                <label class="field"><span>"Zugang"</span>
                    <select prop:value=move || visibility.get().as_str().to_string() on:change=move |ev| visibility.set(WatchVisibility::parse(&event_target_value(&ev)))>
                        <option value="members">{WatchVisibility::Members.label()}</option>
                        <option value="private">{WatchVisibility::Private.label()}</option>
                    </select>
                </label>
                <label class="field"><span>"Playlist"</span>
                    <select prop:value=move || mode.get().as_str().to_string() on:change=move |ev| mode.set(PlaylistMode::parse(&event_target_value(&ev)))>
                        <option value="open">{PlaylistMode::Open.label()}</option>
                        <option value="members">{PlaylistMode::Members.label()}</option>
                        <option value="mods">{PlaylistMode::Mods.label()}</option>
                    </select>
                </label>
                <label class="field"><span>"Voteskip-Schwelle (%)"</span><input type="number" min="1" max="100" bind:value=voteskip/></label>
            </div>
            <div class="form-actions">
                <button class="btn btn-primary" type="submit" disabled=move || saving.get()>{if id.is_some() { "Speichern" } else { "Raum anlegen" }}</button>
            </div>
        </form>
    }
}

// ---------------------------------------------------------------------------
// Room page
// ---------------------------------------------------------------------------

/// Loads a room by slug and renders `child` with it.
#[component]
fn WithRoom(render: Box<dyn Fn(WatchRoomView) -> AnyView + Send + Sync>) -> impl IntoView {
    let params = use_params_map();
    let slug = Memo::new(move |_| params.with(|p| p.get("slug").unwrap_or_default()));
    let room = Resource::new(move || slug.get(), get_watch_room);
    let render = StoredValue::new(render);
    view! {
        <RequireLogin>
            <Suspense fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    match room.await {
                        Err(e) if e.is_not_found() => view! { <Missing text="Diesen Raum gibt es nicht oder du hast keinen Zugang."/> }.into_any(),
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        Ok(r) => render.with_value(|c| c(r)),
                    }
                })}
            </Suspense>
        </RequireLogin>
    }
}

#[component]
pub fn WatchRoomPage() -> impl IntoView {
    view! { <WithRoom render=Box::new(|r| view! { <RoomView room=r/> }.into_any())/> }
}

#[component]
pub fn WatchPlayerPopout() -> impl IntoView {
    view! {
        <WithRoom render=Box::new(|r| {
            let title = format!("▶ {}", r.name);
            let conn = RoomConn::new(r);
            view! {
                <Title text=title/>
                <div class="watch-popout-stage"><Stage conn=conn popout=true/></div>
            }.into_any()
        })/>
    }
}

#[component]
pub fn WatchChatPopout() -> impl IntoView {
    view! {
        <WithRoom render=Box::new(|r| {
            let title = format!("💬 {}", r.name);
            let conn = RoomConn::new(r);
            view! {
                <Title text=title/>
                <div class="watch-popout-chat"><WatchChat conn=conn/></div>
            }.into_any()
        })/>
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Chat,
    Playlist,
    Requests,
    Viewers,
}

fn open_popout(url: &str, name: &str, features: &str) {
    #[cfg(feature = "hydrate")]
    if let Some(w) = web_sys::window() {
        if let Ok(Some(win)) = w.open_with_url_and_target_and_features(url, name, features) {
            let _ = win.focus();
        }
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = (url, name, features);
}

#[component]
fn RoomView(room: WatchRoomView) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let slug = room.slug.clone();
    let conn = RoomConn::new(room);
    let tab = RwSignal::new(SideTab::Chat);
    let seen = RwSignal::new(0usize);
    let settings_open = RwSignal::new(false);
    Effect::new(move |_| {
        let n = conn.chat.with(|c| c.len());
        if tab.get() == SideTab::Chat {
            seen.set(n);
        }
    });
    let unread = move || {
        if tab.get() == SideTab::Chat {
            0
        } else {
            conn.chat.with(|c| c.len()).saturating_sub(seen.get())
        }
    };
    let go = Callback::new(move |p: String| navigate(&p, Default::default()));
    // Reflect playback in the tab title.
    let title = move || {
        let name = conn.room.with(|r| r.as_ref().map(|r| r.name.clone()).unwrap_or_default());
        let playing = conn.state.with(|s| matches!(s, PlaybackState::Playing { .. }));
        if playing { format!("▶ {name}") } else { name }
    };
    let slug_sv = StoredValue::new(slug.clone());
    let copy_link = move |_| {
        copy_to_clipboard(&format!("{}/watch/{}", origin(), slug_sv.get_value()));
        toasts.success("Link kopiert.");
    };
    let popout_player = move |_| {
        let slug = slug_sv.get_value();
        open_popout(&format!("/watch/{slug}/player"), &format!("wcf-player-{slug}"), "popup,width=1280,height=760")
    };
    let popout_chat = move |_: ()| {
        let slug = slug_sv.get_value();
        open_popout(&format!("/watch/{slug}/chat"), &format!("wcf-chat-{slug}"), "popup,width=380,height=720")
    };
    let console_href = StoredValue::new(format!("/watch/{slug}/console"));
    let on_saved = Callback::new(move |r: WatchRoomView| {
        settings_open.set(false);
        conn.room.set(Some(r));
    });
    let can_edit = move || conn.room.with(|r| r.as_ref().is_some_and(|r| r.can_edit));

    view! {
        <Title text=title/>
        <Show when=move || conn.kicked.get().is_none() fallback=move || view! {
            <div class="card center">
                <p class="error">{move || conn.kicked.get().unwrap_or_default()}</p>
                <button class="btn" on:click=move |_| go.run("/watch".into())>"Zurück zu den Räumen"</button>
            </div>
        }>
            <div class="watch-room">
                <section class="watch-main">
                    <div class="watch-head">
                        <a class="muted small" href="/watch"><Icon name="arrow-left" size=14/>"Kino"</a>
                        <h1 class="ellipsis">{move || conn.room.with(|r| r.as_ref().map(|r| r.name.clone()).unwrap_or_default())}</h1>
                        <span class="grow"></span>
                        <span class="muted small"><Icon name="users" size=14/>{move || conn.presence.with(|p| p.len())}</span>
                    </div>
                    <ShowBanner conn=conn/>
                    <Stage conn=conn popout=false/>
                    <div class="stage-tools">
                        <button class="btn btn-ghost btn-sm" on:click=copy_link><Icon name="link" size=14/>"Link kopieren"</button>
                        <button class="btn btn-ghost btn-sm" on:click=popout_player><Icon name="external-link" size=14/>"Player auslagern"</button>
                        <VoteskipButton conn=conn/>
                        <span class="grow"></span>
                        <Show when=can_edit>
                            <button class="btn btn-ghost btn-sm" on:click=move |_| settings_open.set(true)><Icon name="sliders" size=14/>"Raum"</button>
                        </Show>
                    </div>
                    <Show when=move || conn.can_drive()>
                        <OwnerControls conn=conn console_href=console_href.get_value()/>
                    </Show>
                </section>
                <aside class="watch-side card">
                    <div class="tabs side-tabs">
                        <button class="tab" class:active=move || tab.get() == SideTab::Chat on:click=move |_| tab.set(SideTab::Chat)>
                            "Chat"
                            <Show when=move || { unread() > 0 }><span class="tab-badge">{move || unread().min(99)}</span></Show>
                        </button>
                        <button class="tab" class:active=move || tab.get() == SideTab::Playlist on:click=move |_| tab.set(SideTab::Playlist)>
                            "Playlist"<span class="muted small">{move || conn.playlist.with(|p| p.len())}</span>
                        </button>
                        <button class="tab" class:active=move || tab.get() == SideTab::Requests on:click=move |_| tab.set(SideTab::Requests)>"Wünsche"</button>
                        <button class="tab" class:active=move || tab.get() == SideTab::Viewers on:click=move |_| tab.set(SideTab::Viewers)>"Zuschauer"</button>
                    </div>
                    <div class="watch-side-body">
                        {move || match tab.get() {
                            SideTab::Chat => view! { <WatchChat conn=conn on_popout=Callback::new(popout_chat)/> }.into_any(),
                            SideTab::Playlist => view! { <PlaylistPanel conn=conn/> }.into_any(),
                            SideTab::Requests => view! { <RequestsPanel conn=conn manage=false/> }.into_any(),
                            SideTab::Viewers => view! { <ViewersPanel conn=conn/> }.into_any(),
                        }}
                    </div>
                </aside>
            </div>
        </Show>
        <Modal open=settings_open title="Raumeinstellungen" wide=true>
            {move || conn.room.get().map(|r| view! {
                <WatchRoomForm room=Some(r) on_saved=on_saved/>
                <RoomRoles conn=conn/>
                <DeleteRoom conn=conn/>
            })}
        </Modal>
    }
}

#[component]
fn ShowBanner(conn: RoomConn) -> impl IntoView {
    view! {
        {move || {
            let show = conn.show.get()?;
            let media_on_stage = conn.state.with(|s| s.media().is_some());
            (show.status == shared::watch::EventStatus::Scheduled && media_on_stage).then(|| view! {
                <div class="alert small show-banner">
                    <Icon name="calendar" size=14/>
                    {format!("„{}“ beginnt um {} – die Wiedergabe wechselt dann automatisch.", show.title, clock_of(show.starts_at_ms))}
                </div>
            })
        }}
    }
}

// ---------------------------------------------------------------------------
// Stage (break screens in Rust, player in JS)
// ---------------------------------------------------------------------------

#[component]
pub(crate) fn Stage(conn: RoomConn, popout: bool) -> impl IntoView {
    let toasts = use_toasts();
    let offset = conn.offset();
    let tick = ticker(250);
    let role = if popout { "popout" } else { "main" };
    let room_id = conn.room_id;
    let host = NodeRef::<leptos::html::Div>::new();

    // Each stage owns its player instance (a handle returned by the script),
    // so a re-created stage never tears down another one's player.
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::{JsCast, JsValue};
        let handle = StoredValue::new_local(None::<JsValue>);
        let mounted = RwSignal::new(false);
        let call = move |method: &str, args: Vec<JsValue>| {
            handle.try_with_value(|h| {
                if let Some(h) = h {
                    if let Some(f) = js_sys::Reflect::get(h, &method.into())
                        .ok()
                        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
                    {
                        let list = js_sys::Array::new();
                        for a in &args {
                            list.push(a);
                        }
                        let _ = f.apply(h, &list);
                    }
                }
            });
        };
        let to_js = |v: &serde_json::Value| js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL);
        Effect::new(move |_| {
            let Some(el) = host.get() else { return };
            if mounted.get_untracked() {
                return;
            }
            let opts = to_js(&serde_json::json!({ "roomId": room_id, "role": role }));
            leptos::task::spawn_local(async move {
                for _ in 0..50 {
                    let api = web_sys::window().and_then(|w| js_sys::Reflect::get(&w, &"WcfWatch".into()).ok());
                    let mount = api
                        .as_ref()
                        .filter(|a| !a.is_undefined())
                        .and_then(|a| js_sys::Reflect::get(a, &"mount".into()).ok())
                        .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
                    if let (Some(api), Some(mount)) = (api, mount) {
                        if let Ok(h) = mount.call2(&api, &el, &opts) {
                            if !h.is_null() && !h.is_undefined() {
                                handle.set_value(Some(h));
                                mounted.set(true);
                                return;
                            }
                        }
                    }
                    crate::components::common::sleep_ms(100).await;
                }
            });
        });
        Effect::new(move |_| {
            if !mounted.get() {
                return;
            }
            let state = conn.state.get();
            let off = offset.get_untracked();
            call("update", vec![to_js(&serde_json::to_value(&state).unwrap_or_default()), JsValue::from_f64(off)]);
        });
        Effect::new(move |_| {
            let off = offset.get();
            if mounted.get_untracked() {
                call("setOffset", vec![JsValue::from_f64(off)]);
            }
        });
        on_cleanup(move || call("unmount", vec![]));
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = (room_id, role, offset);
    on_player_event(move |ev| match ev {
        PlayerEvent::Duration { key, ms } => conn.send(WatchClientMsg::Duration { room_id, key, duration_ms: ms }),
        PlayerEvent::Error { message } => toasts.error(message),
        PlayerEvent::Audio {} => {}
    });

    let server_now = move || {
        tick.track();
        conn.server_now()
    };
    let countdown = move |target: i64| fmt_timecode(target as f64 - server_now());
    view! {
        <div class="watch-stage" class:popout=popout>
            <div class="watch-player-host" node_ref=host></div>
            {move || match conn.state.get() {
                _ if !conn.welcomed.get() => view! {
                    <div class="break-screen"><Spinner label="Verbinde mit dem Raum …"/></div>
                }.into_any(),
                PlaybackState::Playing { .. } | PlaybackState::Paused { .. } => ().into_any(),
                PlaybackState::Preshow { title, starts_at_ms, poster_url, .. } => view! {
                    <div class="break-screen">
                        {poster_url.map(|p| view! { <img class="break-poster" src=p alt=""/> })}
                        <p class="break-kicker">"Gleich geht's los"</p>
                        <h2>{title}</h2>
                        <div class="count-big">{move || countdown(starts_at_ms)}</div>
                        <p class="muted small">{format!("um {}", clock_of(starts_at_ms))}</p>
                    </div>
                }.into_any(),
                PlaybackState::Intermission { ends_at_ms, next_title, next_poster_url, item_index, item_count, .. } => view! {
                    <div class="break-screen">
                        <p class="break-kicker">"Pause"</p>
                        <div class="count-big">{move || countdown(ends_at_ms)}</div>
                        {next_title.map(|t| view! {
                            {next_poster_url.clone().map(|p| view! { <img class="break-poster" src=p alt=""/> })}
                            <h2>{format!("Als Nächstes: {t}")}</h2>
                            <p class="muted small">{format!("{} von {}", item_index + 2, item_count)}</p>
                        })}
                    </div>
                }.into_any(),
                PlaybackState::Ended { title, .. } => view! {
                    <div class="break-screen">
                        <h2>"Das war's 🎬"</h2>
                        <p class="muted">{title}</p>
                    </div>
                }.into_any(),
                PlaybackState::Idle => view! {
                    <div class="break-screen idle">
                        <Icon name="tv" size=42/>
                        <h2>"Gerade läuft nichts"</h2>
                        <p class="muted">"Die Leinwand leuchtet auf, sobald etwas startet – füge etwas zur Playlist hinzu."</p>
                    </div>
                }.into_any(),
            }}
        </div>
    }
}

#[component]
fn VoteskipButton(conn: RoomConn) -> impl IntoView {
    let visible = move || conn.current_item.get().is_some() && !conn.show_live();
    view! {
        <Show when=visible>
            <button class="btn btn-ghost btn-sm" class:active=move || conn.voted.get()
                title="Stimme für das Überspringen des aktuellen Videos"
                on:click=move |_| {
                    conn.voted.update(|v| *v = !*v);
                    conn.send(WatchClientMsg::Voteskip { room_id: conn.room_id });
                }>
                <Icon name="skip-forward" size=14/>
                {move || { let (v, n) = conn.voteskip.get(); format!("Überspringen {v}/{n}") }}
            </button>
        </Show>
    }
}

// ---------------------------------------------------------------------------
// Transport for owner / mods / leader
// ---------------------------------------------------------------------------

#[component]
pub(crate) fn Transport(conn: RoomConn) -> impl IntoView {
    let tick = ticker(500);
    let dragging = RwSignal::new(None::<f64>);
    let pos = move || {
        tick.track();
        let now = conn.server_now();
        conn.state.with(|s| {
            let p = s.position_at(now);
            match s.media().and_then(|m| m.duration_ms) {
                Some(d) => p.min(d as f64),
                None => p,
            }
        })
    };
    let duration = move || conn.state.with(|s| s.media().and_then(|m| m.duration_ms));
    let playing = move || conn.state.with(|s| matches!(s, PlaybackState::Playing { .. }));
    view! {
        <div class="transport">
            {move || if playing() {
                view! { <button class="btn btn-sm" on:click=move |_| conn.control(ControlAction::Pause, |_| {})><Icon name="pause" size=14/>"Pause"</button> }.into_any()
            } else {
                view! { <button class="btn btn-sm" on:click=move |_| conn.control(ControlAction::Play, |_| {})><Icon name="play" size=14/>"Weiter"</button> }.into_any()
            }}
            <input class="seek grow" type="range" min="0" step="1000"
                prop:max=move || duration().unwrap_or(0).to_string()
                disabled=move || duration().is_none()
                prop:value=move || dragging.get().unwrap_or_else(pos).to_string()
                on:input=move |ev| dragging.set(event_target_value(&ev).parse().ok())
                on:change=move |ev| {
                    let ms: f64 = event_target_value(&ev).parse().unwrap_or(0.0);
                    dragging.set(None);
                    conn.control(ControlAction::Seek, |a| a.position_ms = Some(ms));
                }/>
            <span class="muted small mono">{move || {
                let p = dragging.get().unwrap_or_else(pos);
                match duration() {
                    Some(d) => format!("{} / {}", fmt_timecode(p), fmt_timecode(d as f64)),
                    None => fmt_timecode(p),
                }
            }}</span>
        </div>
    }
}

#[component]
fn OwnerControls(conn: RoomConn, console_href: String) -> impl IntoView {
    let library = LocalResource::new(move || async move { watch_library().await.map(|l| l.media).unwrap_or_default() });
    let pick = RwSignal::new(String::new());
    let has_media = move || conn.state.with(|s| s.media().is_some());
    let controller = move || conn.can_control();
    let console = console_href.clone();
    view! {
        <div class="owner-controls card">
            {move || if has_media() {
                view! {
                    <Transport conn=conn/>
                    <div class="row gap-sm">
                        <Show when=move || conn.current_item.get().is_some() && !conn.show_live()>
                            <button class="btn btn-sm" on:click=move |_| conn.control(ControlAction::Next, |_| {})><Icon name="skip-forward" size=14/>"Nächstes"</button>
                        </Show>
                        <Show when=move || controller() && !conn.show_live()>
                            <button class="btn btn-sm btn-ghost" on:click=move |_| conn.control(ControlAction::Stop, |_| {})><Icon name="x" size=14/>"Stopp"</button>
                        </Show>
                    </div>
                }.into_any()
            } else if conn.show_live() {
                view! { <span class="muted small">"Eine Vorstellung läuft – steuere sie in der Konsole."</span> }.into_any()
            } else {
                view! {
                    <Show when=controller fallback=|| view! { <span class="muted small">"Starte die Playlist, indem du etwas hinzufügst."</span> }>
                        <div class="row gap-sm grow">
                            <select class="grow" prop:value=move || pick.get() on:change=move |ev| pick.set(event_target_value(&ev))>
                                <option value="">"Film aus der Mediathek …"</option>
                                {move || local(library).unwrap_or_default().into_iter().filter(|m| m.status == shared::watch::MediaStatus::Ready).map(|m| view! {
                                    <option value=m.id.to_string()>{m.title.clone()}</option>
                                }).collect_view()}
                            </select>
                            <button class="btn btn-sm btn-primary" disabled=move || pick.get().is_empty()
                                on:click=move |_| {
                                    if let Ok(id) = Uuid::parse_str(&pick.get_untracked()) {
                                        conn.control(ControlAction::PlayNow, |a| a.media_id = Some(id));
                                    }
                                }><Icon name="play" size=14/>"Jetzt abspielen"</button>
                        </div>
                    </Show>
                }.into_any()
            }}
            <Show when=controller>
                <a class="btn btn-sm btn-ghost" href=console.clone() target="_blank"><Icon name="sliders" size=14/>"Konsole"</a>
            </Show>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Chat with emotes and mentions
// ---------------------------------------------------------------------------

enum Token {
    Text(String),
    Emote(String, String),
    Mention(String, bool),
}

/// Splits a message into text, `:emote:` and `@mention` tokens. Never HTML.
fn tokenize(body: &str, emotes: &HashMap<String, String>, me: Option<&str>) -> Vec<Token> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    while i < chars.len() {
        if chars[i] == ':' {
            let mut j = i + 1;
            while j < chars.len() && j - i <= 25 && (chars[j].is_ascii_lowercase() || chars[j].is_ascii_digit() || chars[j] == '_') {
                j += 1;
            }
            if j < chars.len() && chars[j] == ':' && j - i > 2 {
                let code: String = chars[i + 1..j].iter().collect();
                if let Some(url) = emotes.get(&code) {
                    if !text.is_empty() {
                        out.push(Token::Text(std::mem::take(&mut text)));
                    }
                    out.push(Token::Emote(code, url.clone()));
                    i = j + 1;
                    continue;
                }
            }
        } else if chars[i] == '@' && (i == 0 || !word(chars[i - 1])) {
            let mut j = i + 1;
            while j < chars.len() && word(chars[j]) && j - i <= 32 {
                j += 1;
            }
            if j - i > 2 {
                let name: String = chars[i + 1..j].iter().collect();
                if !text.is_empty() {
                    out.push(Token::Text(std::mem::take(&mut text)));
                }
                let mine = me.is_some_and(|m| m.eq_ignore_ascii_case(&name));
                out.push(Token::Mention(name, mine));
                i = j;
                continue;
            }
        }
        text.push(chars[i]);
        i += 1;
    }
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    out
}

fn name_hue(name: &str) -> u32 {
    name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360
}

#[component]
pub(crate) fn WatchChat(conn: RoomConn, #[prop(optional)] on_popout: Option<Callback<()>>) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let emotes = LocalResource::new(|| async { watch_emotes().await.unwrap_or_default() });
    let emote_map = Memo::new(move |_| {
        local(emotes)
            .unwrap_or_default()
            .into_iter()
            .map(|e: Emote| (e.code, e.url))
            .collect::<HashMap<_, _>>()
    });
    let text = RwSignal::new(String::new());
    let picker = RwSignal::new(false);
    let list_ref = NodeRef::<leptos::html::Div>::new();
    let stick = RwSignal::new(true);
    let unseen = RwSignal::new(0usize);
    let tick = ticker(1000);
    let me_name = move || session.user.with(|u| u.as_ref().map(|u| u.username.clone()));

    let scroll_bottom = move || {
        if let Some(el) = list_ref.get_untracked() {
            el.set_scroll_top(el.scroll_height());
        }
    };
    Effect::new(move |prev: Option<usize>| {
        let n = conn.chat.with(|c| c.len());
        if stick.get_untracked() {
            #[cfg(feature = "hydrate")]
            leptos::task::spawn_local(async move {
                crate::components::common::sleep_ms(10).await;
                scroll_bottom();
            });
        } else if prev.is_some_and(|p| n > p) {
            unseen.update(|u| *u += n - prev.unwrap_or(n));
        }
        n
    });
    let on_scroll = move |_| {
        if let Some(el) = list_ref.get_untracked() {
            let near = el.scroll_height() - el.scroll_top() - el.client_height() < 60;
            stick.set(near);
            if near {
                unseen.set(0);
            }
        }
    };
    let send = move || {
        let body = text.get_untracked();
        if body.trim().is_empty() {
            return;
        }
        conn.send(WatchClientMsg::Chat { room_id: conn.room_id, body });
        text.set(String::new());
        stick.set(true);
    };
    let delete = move |id: Uuid| {
        leptos::task::spawn_local(async move {
            if let Err(e) = delete_watch_chat(conn.room_id, id).await {
                toasts.error(e.to_string());
            }
        });
    };
    let chat_error = move || {
        tick.track();
        conn.last_error.get().filter(|(code, _, at)| {
            matches!(code.as_str(), "muted" | "slow_mode" | "rate_limited") && now_ms() - at < 8000.0
        })
    };
    let msgs = move || conn.chat.get();

    view! {
        <div class="watch-chat">
            <div class="watch-chat-head">
                <span class="muted small"><Icon name="users" size=13/>{move || format!("{} im Raum", conn.presence.with(|p| p.len()))}</span>
                <span class="grow"></span>
                {on_popout.map(|cb| view! {
                    <button class="btn btn-ghost btn-sm" title="Chat in eigenem Fenster" on:click=move |_| cb.run(())><Icon name="external-link" size=13/></button>
                })}
            </div>
            <div class="watch-chat-log" node_ref=list_ref on:scroll=on_scroll>
                <For each=msgs key=|m| m.id children=move |m: WatchChatMessage| {
                    let me = me_name();
                    let own = me.as_deref() == Some(m.user.username.as_str());
                    let tokens = tokenize(&m.body, &emote_map.get_untracked(), me.as_deref());
                    let mentioned = !own && tokens.iter().any(|t| matches!(t, Token::Mention(_, true)));
                    let id = m.id;
                    let hue = name_hue(&m.user.username);
                    let at = clock_of(m.at_ms);
                    view! {
                        <div class="chat-line" class:mention=mentioned>
                            <span class="chat-time muted">{at}</span>
                            <a class="chat-name" class:own=own href=m.user.profile_url() style=format!("--hue:{hue}")>{m.user.display_name.clone()}</a>
                            <span class="chat-body">
                                {tokens.into_iter().map(|t| match t {
                                    Token::Text(s) => view! { <span>{s}</span> }.into_any(),
                                    Token::Emote(code, url) => view! { <img class="emote" src=url alt=format!(":{code}:") title=format!(":{code}:")/> }.into_any(),
                                    Token::Mention(n, mine) => view! { <span class="chat-mention" class:own-mention=mine>{format!("@{n}")}</span> }.into_any(),
                                }).collect_view()}
                            </span>
                            <Show when=move || conn.can_control()>
                                <button class="chat-del" title="Nachricht löschen" on:click=move |_| delete(id)><Icon name="x" size=12/></button>
                            </Show>
                        </div>
                    }
                }/>
            </div>
            <Show when=move || { unseen.get() > 0 }>
                <button class="chat-unseen" on:click=move |_| { stick.set(true); unseen.set(0); scroll_bottom(); }>
                    {move || format!("{} neue Nachrichten ↓", unseen.get())}
                </button>
            </Show>
            {move || chat_error().map(|(_, msg, _)| view! { <div class="chat-error small">{msg}</div> })}
            <Show when=move || picker.get() && !emote_map.with(|m| m.is_empty())>
                <div class="emote-picker">
                    {move || {
                        let mut list: Vec<(String, String)> = emote_map.get().into_iter().collect();
                        list.sort();
                        list.into_iter().map(|(code, url)| {
                            let c = code.clone();
                            view! {
                                <button class="emote-pick" title=format!(":{code}:") on:click=move |_| {
                                    text.update(|t| {
                                        if !t.is_empty() && !t.ends_with(' ') { t.push(' '); }
                                        t.push_str(&format!(":{c}: "));
                                    });
                                    picker.set(false);
                                }><img class="emote" src=url alt=""/></button>
                            }
                        }).collect_view()
                    }}
                </div>
            </Show>
            <form class="watch-chat-form" on:submit=move |ev: leptos::ev::SubmitEvent| { ev.prevent_default(); send(); }>
                <Show when=move || !emote_map.with(|m| m.is_empty())>
                    <button type="button" class="btn btn-ghost btn-sm" title="Emotes" on:click=move |_| picker.update(|p| *p = !*p)><Icon name="smile" size=16/></button>
                </Show>
                <input type="text" class="grow" maxlength=CHAT_MAX_CHARS.to_string() bind:value=text
                    placeholder=move || {
                        let ms = conn.slow_mode.get();
                        if ms > 0 { format!("Slow-Mode: alle {} s eine Nachricht", ms / 1000) } else { "Nachricht …".to_string() }
                    }/>
                <button class="btn btn-primary btn-sm" type="submit"><Icon name="send" size=14/></button>
            </form>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------

#[component]
fn PlaylistPanel(conn: RoomConn) -> impl IntoView {
    let toasts = use_toasts();
    let url = RwSignal::new(String::new());
    let next = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let source_tab = RwSignal::new(0u8);
    let lib_pick = RwSignal::new(String::new());
    let file_pick = RwSignal::new(String::new());
    let library = LocalResource::new(move || async move {
        watch_library().await.map(|l| l.media).unwrap_or_default()
    });
    let videos = LocalResource::new(move || async move { my_watch_videos().await.unwrap_or_default() });
    let can_add = move || conn.you.with(|y| y.as_ref().is_some_and(|y| y.can_add));
    let me = use_session().user_id();

    let add = move |source: PlaylistSource| {
        busy.set(true);
        let nx = next.get_untracked();
        leptos::task::spawn_local(async move {
            match add_watch_item(conn.room_id, source, nx).await {
                Ok(()) => {
                    url.set(String::new());
                    toasts.success("Zur Playlist hinzugefügt.");
                }
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    };
    let run = move |fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::error::AppError>>>>| {
        leptos::task::spawn_local(async move {
            if let Err(e) = fut.await {
                toasts.error(e.to_string());
            }
        });
    };
    let items = move || conn.playlist.get();
    let room_id = conn.room_id;

    view! {
        <div class="playlist-panel">
            <Show when=can_add fallback=|| view! { <p class="muted small">"In diesem Raum füllen nur Moderatoren die Playlist."</p> }>
                <div class="playlist-add">
                    <div class="seg small">
                        <button class:active=move || source_tab.get() == 0 on:click=move |_| source_tab.set(0)>"Link"</button>
                        <button class:active=move || source_tab.get() == 1 on:click=move |_| source_tab.set(1)>"Mediathek"</button>
                        <button class:active=move || source_tab.get() == 2 on:click=move |_| source_tab.set(2)>"Meine Dateien"</button>
                    </div>
                    {move || match source_tab.get() {
                        0 => view! {
                            <form class="row gap-sm" on:submit=move |ev: leptos::ev::SubmitEvent| {
                                ev.prevent_default();
                                let u = url.get_untracked();
                                if !u.trim().is_empty() { add(PlaylistSource::Url { url: u }); }
                            }>
                                <input class="grow" type="url" placeholder="YouTube-Link oder Video-URL (mp4, webm, m3u8)" bind:value=url/>
                                <button class="btn btn-sm btn-primary" type="submit" disabled=move || busy.get()><Icon name="plus" size=14/></button>
                            </form>
                        }.into_any(),
                        1 => view! {
                            <div class="row gap-sm">
                                <select class="grow" prop:value=move || lib_pick.get() on:change=move |ev| lib_pick.set(event_target_value(&ev))>
                                    <option value="">"Film wählen …"</option>
                                    {move || local(library).unwrap_or_default().into_iter().filter(|m| m.status == shared::watch::MediaStatus::Ready).map(|m| view! {
                                        <option value=m.id.to_string()>{m.title.clone()}</option>
                                    }).collect_view()}
                                </select>
                                <button class="btn btn-sm btn-primary" disabled=move || lib_pick.get().is_empty() || busy.get()
                                    on:click=move |_| if let Ok(id) = Uuid::parse_str(&lib_pick.get_untracked()) { add(PlaylistSource::Library { media_id: id }); }>
                                    <Icon name="plus" size=14/>
                                </button>
                            </div>
                        }.into_any(),
                        _ => view! {
                            <div class="row gap-sm">
                                <select class="grow" prop:value=move || file_pick.get() on:change=move |ev| file_pick.set(event_target_value(&ev))>
                                    <option value="">"Video aus „Dateien“ …"</option>
                                    {move || local(videos).unwrap_or_default().into_iter().map(|f| view! {
                                        <option value=f.id.to_string()>{f.name.clone()}</option>
                                    }).collect_view()}
                                </select>
                                <button class="btn btn-sm btn-primary" disabled=move || file_pick.get().is_empty() || busy.get()
                                    on:click=move |_| if let Ok(id) = Uuid::parse_str(&file_pick.get_untracked()) { add(PlaylistSource::File { file_id: id }); }>
                                    <Icon name="plus" size=14/>
                                </button>
                            </div>
                        }.into_any(),
                    }}
                    <Show when=move || conn.can_drive()>
                        <label class="check small"><input type="checkbox" bind:checked=next/><span>"Als Nächstes einreihen"</span></label>
                    </Show>
                </div>
            </Show>
            <ol class="playlist">
                {move || {
                    let list = items();
                    if list.is_empty() {
                        return view! { <li class="muted small center">"Die Playlist ist leer."</li> }.into_any();
                    }
                    let current = conn.current_item.get();
                    let ids: Vec<Uuid> = list.iter().map(|i| i.id).collect();
                    list.into_iter().enumerate().map(|(n, item)| {
                        let id = item.id;
                        let is_current = current == Some(id);
                        let own = item.added_by.as_ref().map(|u| u.id) == me;
                        let prev = (n > 0).then(|| ids[n - 1]);
                        let after_next = ids.get(n + 2).copied();
                        let is_last = n + 1 == ids.len();
                        view! {
                            <li class="playlist-item" class:current=is_current>
                                {item.thumb_url.clone().map(|t| view! { <img class="playlist-thumb" src=t alt="" loading="lazy"/> })}
                                <div class="grow playlist-meta">
                                    <div class="ellipsis" title=item.title.clone()>
                                        {is_current.then(|| view! { <span class="live-badge small">"läuft"</span> " " })}
                                        {item.title.clone()}
                                    </div>
                                    <div class="muted small">
                                        {item.kind.label()}
                                        {item.duration_ms.map(|d| format!(" · {}", fmt_timecode(d as f64)))}
                                        {item.added_by.as_ref().map(|u| format!(" · {}", u.display_name))}
                                    </div>
                                </div>
                                <div class="playlist-tools">
                                    <Show when=move || conn.can_drive() && !is_current && !conn.show_live()>
                                        <button class="icon-btn" title="Jetzt abspielen" on:click=move |_| conn.control(ControlAction::PlayItem, |a| a.item_id = Some(id))><Icon name="play" size=14/></button>
                                    </Show>
                                    <Show when=move || conn.can_drive() && !is_current>
                                        <button class="icon-btn" title="Nach oben" disabled=prev.is_none()
                                            on:click=move |_| run(Box::pin(move_watch_item(room_id, id, prev)))><Icon name="arrow-up" size=14/></button>
                                        <button class="icon-btn" title="Nach unten" disabled=is_last
                                            on:click=move |_| run(Box::pin(move_watch_item(room_id, id, after_next)))><Icon name="arrow-down" size=14/></button>
                                    </Show>
                                    <Show when=move || conn.can_drive() || own>
                                        <button class="icon-btn" title="Entfernen" on:click=move |_| run(Box::pin(remove_watch_item(room_id, id)))><Icon name="x" size=14/></button>
                                    </Show>
                                </div>
                            </li>
                        }
                    }).collect_view().into_any()
                }}
            </ol>
            <Show when=move || conn.can_drive() && !conn.playlist.with(|p| p.is_empty())>
                <div class="row gap-sm">
                    <button class="btn btn-sm btn-ghost" on:click=move |_| run(Box::pin(shuffle_watch_playlist(room_id)))><Icon name="shuffle" size=14/>"Mischen"</button>
                    <button class="btn btn-sm btn-ghost" on:click=move |_| if confirm("Playlist leeren (außer dem laufenden Video)?") { run(Box::pin(clear_watch_playlist(room_id))) }><Icon name="trash" size=14/>"Leeren"</button>
                </div>
            </Show>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Requests (proposals)
// ---------------------------------------------------------------------------

#[component]
pub(crate) fn RequestsPanel(
    conn: RoomConn,
    manage: bool,
    #[prop(optional)] on_promote: Option<Callback<shared::watch::ProposalInfo>>,
) -> impl IntoView {
    let toasts = use_toasts();
    let me = use_session().user_id();
    let list = LocalResource::new(move || {
        conn.proposals_version.track();
        async move { watch_proposals(conn.room_id, manage).await.unwrap_or_default() }
    });
    let title = RwSignal::new(String::new());
    let link = RwSignal::new(String::new());
    let notes = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        let input = ProposalInput { title: title.get_untracked(), url: link.get_untracked(), notes: notes.get_untracked() };
        leptos::task::spawn_local(async move {
            match create_watch_proposal(conn.room_id, input).await {
                Ok(()) => {
                    title.set(String::new());
                    link.set(String::new());
                    notes.set(String::new());
                }
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    };
    let act = move |fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::error::AppError>>>>| {
        leptos::task::spawn_local(async move {
            if let Err(e) = fut.await {
                toasts.error(e.to_string());
            }
        });
    };
    view! {
        <div class="requests">
            {(!manage).then(|| view! {
                <form class="form compact" on:submit=submit>
                    <input type="text" maxlength="120" required placeholder="Filmwunsch (Titel)" bind:value=title/>
                    <input type="url" maxlength="300" placeholder="Link (optional, z. B. IMDb)" bind:value=link/>
                    <input type="text" maxlength="500" placeholder="Notiz (optional)" bind:value=notes/>
                    <button class="btn btn-sm btn-primary" type="submit" disabled=move || busy.get()><Icon name="plus" size=14/>"Wünschen"</button>
                </form>
            })}
            <ul class="plain-list request-list">
                {move || {
                    let items = local(list).unwrap_or_default();
                    if items.is_empty() {
                        return view! { <li class="muted small center">"Noch keine Wünsche."</li> }.into_any();
                    }
                    items.into_iter().map(|p| {
                        let id = p.id;
                        let own = Some(p.created_by.id) == me;
                        let open = p.status == ProposalStatus::Open;
                        let promote_item = p.clone();
                        view! {
                            <li class="request-item">
                                <button class="vote-btn" class:voted=p.my_vote disabled=!open title="Abstimmen"
                                    on:click=move |_| act(Box::pin(vote_watch_proposal(id)))>
                                    <Icon name="arrow-up" size=14/><span>{p.votes}</span>
                                </button>
                                <div class="grow">
                                    <div>
                                        {match p.url.clone() {
                                            Some(u) => view! { <a href=u target="_blank" rel="noopener noreferrer">{p.title.clone()}</a> }.into_any(),
                                            None => view! { <strong>{p.title.clone()}</strong> }.into_any(),
                                        }}
                                        {(!open).then(|| view! { " " <span class="tag-chip">{p.status.label()}</span> })}
                                    </div>
                                    {p.notes.clone().map(|n| view! { <div class="muted small">{n}</div> })}
                                    <div class="muted small">{format!("von {}", p.created_by.display_name)}</div>
                                </div>
                                {(own && open && !manage).then(|| view! {
                                    <button class="icon-btn" title="Zurückziehen" on:click=move |_| act(Box::pin(delete_watch_proposal(id)))><Icon name="x" size=14/></button>
                                })}
                                {manage.then(|| view! {
                                    <div class="row gap-sm">
                                        {on_promote.filter(|_| open).map(|cb| view! {
                                            <button class="btn btn-sm" title="Als Vorstellung planen" on:click=move |_| cb.run(promote_item.clone())><Icon name="calendar" size=13/>"Planen"</button>
                                        })}
                                        {if open {
                                            view! { <button class="btn btn-sm btn-ghost" on:click=move |_| act(Box::pin(set_watch_proposal_status(id, ProposalStatus::Rejected)))>"Ablehnen"</button> }.into_any()
                                        } else {
                                            view! { <button class="btn btn-sm btn-ghost" on:click=move |_| act(Box::pin(set_watch_proposal_status(id, ProposalStatus::Open)))>"Wieder öffnen"</button> }.into_any()
                                        }}
                                        <button class="icon-btn" title="Löschen" on:click=move |_| act(Box::pin(delete_watch_proposal(id)))><Icon name="trash" size=14/></button>
                                    </div>
                                })}
                            </li>
                        }
                    }).collect_view().into_any()
                }}
            </ul>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Viewers + moderation
// ---------------------------------------------------------------------------

#[component]
pub(crate) fn ViewersPanel(conn: RoomConn) -> impl IntoView {
    let toasts = use_toasts();
    let session = use_session();
    let me = session.user_id();
    let restricted_version = RwSignal::new(0u32);
    let restricted = LocalResource::new(move || {
        restricted_version.track();
        let ctl = conn.can_control();
        async move {
            if ctl {
                watch_restricted(conn.room_id).await.unwrap_or_default()
            } else {
                Vec::new()
            }
        }
    });
    let moderate = move |user_id: Uuid, kind: ModerationKind, minutes: Option<i64>| {
        if kind == ModerationKind::Ban && !confirm("Nutzer aus diesem Raum verbannen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match moderate_watch_user(conn.room_id, user_id, kind, minutes, String::new()).await {
                Ok(()) => {
                    toasts.success("Erledigt.");
                    restricted_version.update(|v| *v += 1);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let set_mod = move |user_id: Uuid, role: Option<RoomRole>| {
        leptos::task::spawn_local(async move {
            if let Err(e) = set_watch_role(conn.room_id, user_id, role).await {
                toasts.error(e.to_string());
            }
        });
    };
    let slow_options: [(i64, &str); 6] = [(0, "Aus"), (5000, "5 s"), (10000, "10 s"), (30000, "30 s"), (60000, "60 s"), (120000, "120 s")];
    let is_owner = move || {
        conn.you.with(|y| y.as_ref().is_some_and(|y| y.role == Some(RoomRole::Owner)))
            || session.is_admin()
    };
    view! {
        <div class="viewers">
            <Show when=move || conn.can_control()>
                <label class="field row gap-sm">
                    <span class="small">"Slow-Mode"</span>
                    <select prop:value=move || conn.slow_mode.get().to_string()
                        on:change=move |ev| {
                            let ms: i64 = event_target_value(&ev).parse().unwrap_or(0);
                            leptos::task::spawn_local(async move {
                                if let Err(e) = set_watch_slow_mode(conn.room_id, ms).await { toasts.error(e.to_string()); }
                            });
                        }>
                        {slow_options.iter().map(|(v, l)| view! { <option value=v.to_string()>{*l}</option> }).collect_view()}
                    </select>
                </label>
            </Show>
            <ul class="plain-list viewer-list">
                {move || conn.presence.get().into_iter().map(|p| {
                    let uid = p.user.id;
                    let is_me = Some(uid) == me;
                    let role = p.role;
                    view! {
                        <li class="viewer">
                            <Avatar user=p.user.clone() size=26/>
                            <a class="grow ellipsis user-link" href=p.user.profile_url()>{p.user.display_name.clone()}</a>
                            {p.leader.then(|| view! { <span class="tag-chip" title="Leader">"👑 Leader"</span> })}
                            {role.filter(|r| *r != RoomRole::Member).map(|r| view! { <span class="tag-chip">{r.label()}</span> })}
                            <Show when=move || conn.can_control() && !is_me>
                                <crate::components::Dropdown align_right=true label="Aktionen" trigger=|| view! { <span class="icon-btn"><Icon name="more-vertical" size=15/></span> }>
                                    <button class="menu-item" on:click=move |_| {
                                        let leader = conn.presence.with_untracked(|ps| ps.iter().any(|x| x.user.id == uid && x.leader));
                                        conn.control(ControlAction::SetLeader, |a| a.user_id = (!leader).then_some(uid));
                                    }><Icon name="star" size=14/>"Leader geben/nehmen"</button>
                                    <button class="menu-item" on:click=move |_| moderate(uid, ModerationKind::Timeout, Some(5))><Icon name="clock" size=14/>"5 Min. stumm"</button>
                                    <button class="menu-item" on:click=move |_| moderate(uid, ModerationKind::Timeout, Some(30))><Icon name="clock" size=14/>"30 Min. stumm"</button>
                                    <button class="menu-item danger" on:click=move |_| moderate(uid, ModerationKind::Ban, None)><Icon name="user-x" size=14/>"Aus dem Raum verbannen"</button>
                                    <Show when=is_owner>
                                        {if role == Some(RoomRole::Mod) {
                                            view! { <button class="menu-item" on:click=move |_| set_mod(uid, None)><Icon name="shield" size=14/>"Moderator entziehen"</button> }.into_any()
                                        } else {
                                            view! { <button class="menu-item" on:click=move |_| set_mod(uid, Some(RoomRole::Mod))><Icon name="shield" size=14/>"Zum Moderator machen"</button> }.into_any()
                                        }}
                                    </Show>
                                </crate::components::Dropdown>
                            </Show>
                        </li>
                    }
                }).collect_view()}
            </ul>
            <Show when=move || !local(restricted).unwrap_or_default().is_empty()>
                <h3 class="small">"Eingeschränkt"</h3>
                <ul class="plain-list viewer-list">
                    {move || local(restricted).unwrap_or_default().into_iter().map(|r| {
                        let uid = r.user.id;
                        let what = if r.banned { "verbannt".to_string() } else {
                            r.until_ms.map(|u| format!("stumm bis {}", clock_of(u))).unwrap_or_default()
                        };
                        view! {
                            <li class="viewer">
                                <Avatar user=r.user.clone() size=22/>
                                <span class="grow ellipsis">{r.user.display_name.clone()}</span>
                                <span class="muted small">{what}</span>
                                <button class="btn btn-sm btn-ghost" on:click=move |_| moderate(uid, ModerationKind::Unban, None)>"Aufheben"</button>
                            </li>
                        }
                    }).collect_view()}
                </ul>
            </Show>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Room settings: roles + delete
// ---------------------------------------------------------------------------

#[component]
fn RoomRoles(conn: RoomConn) -> impl IntoView {
    let toasts = use_toasts();
    let version = RwSignal::new(0u32);
    let roles = LocalResource::new(move || {
        version.track();
        async move { watch_room_roles(conn.room_id).await.unwrap_or_default() }
    });
    let picked = RwSignal::new(Vec::<UserSummary>::new());
    let new_role = RwSignal::new("member".to_string());
    let set = move |user_id: Uuid, role: Option<RoomRole>| {
        leptos::task::spawn_local(async move {
            match set_watch_role(conn.room_id, user_id, role).await {
                Ok(()) => version.update(|v| *v += 1),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <div class="room-members">
            <h3>"Moderatoren & Mitglieder"</h3>
            <p class="muted small">"Moderatoren steuern die Wiedergabe und moderieren. Mitglieder haben Zugang zu privaten Räumen bzw. dürfen bei „Raummitglieder“ die Playlist füllen."</p>
            <ul class="plain-list">
                {move || local(roles).unwrap_or_default().into_iter().map(|r| {
                    let uid = r.user.id;
                    view! {
                        <li class="row gap-sm member-row">
                            <Avatar user=r.user.clone() size=24/>
                            <span class="grow">{r.user.display_name.clone()}</span>
                            <span class="tag-chip">{r.role.label()}</span>
                            <button class="icon-btn" title="Entfernen" on:click=move |_| set(uid, None)><Icon name="x" size=14/></button>
                        </li>
                    }
                }).collect_view()}
            </ul>
            <div class="row gap-sm">
                <div class="grow"><UserPicker selected=picked placeholder="Nutzer hinzufügen …"/></div>
                <select prop:value=move || new_role.get() on:change=move |ev| new_role.set(event_target_value(&ev))>
                    <option value="member">"Mitglied"</option>
                    <option value="mod">"Moderator"</option>
                </select>
                <button class="btn" disabled=move || picked.with(|p| p.is_empty()) on:click=move |_| {
                    let role = RoomRole::parse(&new_role.get_untracked());
                    for u in picked.get_untracked() { set(u.id, role); }
                    picked.set(Vec::new());
                }>"Hinzufügen"</button>
            </div>
        </div>
    }
}

#[component]
fn DeleteRoom(conn: RoomConn) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |p: String| navigate(&p, Default::default()));
    view! {
        <div class="danger-zone">
            <button class="btn btn-danger-soft" on:click=move |_| {
                if !confirm("Raum mit Playlist, Chat und Vorstellungen endgültig löschen?") { return; }
                leptos::task::spawn_local(async move {
                    match delete_watch_room(conn.room_id).await {
                        Ok(()) => go.run("/watch".into()),
                        Err(e) => toasts.error(e.to_string()),
                    }
                });
            }><Icon name="trash" size=15/>"Raum löschen"</button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_emotes_and_mentions() {
        let mut emotes = HashMap::new();
        emotes.insert("popcorn".to_string(), "/files/x".to_string());
        let t = tokenize("Los geht's :popcorn: @alice und :unknown: <b>", &emotes, Some("alice"));
        assert!(matches!(&t[1], Token::Emote(c, _) if c == "popcorn"));
        assert!(t.iter().any(|x| matches!(x, Token::Mention(n, true) if n == "alice")));
        // Unknown codes and markup stay plain text.
        assert!(matches!(t.last(), Some(Token::Text(s)) if s.contains(":unknown:") && s.contains("<b>")));
    }
}
