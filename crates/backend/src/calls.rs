//! Calls: persistent and spontaneous rooms, 1:1 calls with ringing, live
//! participant lists (realtime topic `Call(id)`) and moderation. Media runs
//! through LiveKit; this module only decides who may join and hands out
//! tokens.

use std::time::Duration as StdDuration;

use serde_json::json;
use shared::call::{
    CallEvent, CallRoomDetail, CallRoomInput, CallRoomView, CallStatus, CallsOverview, DirectCallDetail,
    DirectCallView, JoinInfo, RoomMember, RoomVisibility, MODULE, RING_SECONDS,
};
use shared::ws::{ServerMsg, Topic};
use shared::{AuthUser, NotificationKind};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::livekit::{self, Grant};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::users::{user_cols, user_from};

const TOKEN_TTL: StdDuration = StdDuration::from_secs(6 * 3600);
/// Spontaneous rooms are removed after being empty this long.
const SPONTANEOUS_IDLE_MINUTES: i32 = 10;

fn emit(payload: &CallEvent) -> ServerMsg {
    ServerMsg::Module {
        module: MODULE.to_string(),
        payload: serde_json::to_value(payload).unwrap_or_default(),
    }
}

fn require_configured(state: &AppState) -> Result<&crate::config::LiveKitConfig> {
    if !state.settings().modules.calls {
        return Err(Error::invalid("Anrufe sind deaktiviert."));
    }
    state
        .config
        .livekit
        .as_ref()
        .ok_or_else(|| Error::invalid("Anrufe sind auf diesem Server nicht eingerichtet (LiveKit fehlt)."))
}

fn join_info(state: &AppState, user: &AuthUser, lk_room: String, moderator: bool) -> Result<JoinInfo> {
    let cfg = require_configured(state)?;
    let summary = user.summary();
    let metadata = json!({ "avatar": summary.avatar_url, "username": summary.username });
    let token = livekit::participant_token(
        cfg,
        &user.id.to_string(),
        &summary.display_name,
        &metadata,
        &Grant { room: lk_room, admin: moderator, can_publish: true },
        TOKEN_TTL,
    );
    Ok(JoinInfo {
        url: cfg.url.clone(),
        token,
        identity: user.id.to_string(),
        moderator,
    })
}

fn room_lk_name(id: Uuid) -> String {
    format!("room-{id}")
}

fn direct_lk_name(id: Uuid) -> String {
    format!("dm-{id}")
}

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

fn room_select() -> String {
    format!(
        "SELECT r.id, r.name, r.description, r.visibility, r.persistent, r.created_at, r.last_active_at, \
           r.owner_id, {} FROM call_rooms r JOIN users u ON u.id = r.owner_id ",
        user_cols("u", "owner")
    )
}

struct RoomRow {
    view: CallRoomView,
}

async fn member_role(db: &PgPool, room_id: Uuid, user_id: Uuid) -> Result<Option<String>> {
    Ok(sqlx::query_scalar("SELECT role FROM call_room_members WHERE room_id = $1 AND user_id = $2")
        .bind(room_id)
        .bind(user_id)
        .fetch_optional(db)
        .await?)
}

async fn room_from(state: &AppState, r: &PgRow, viewer: &AuthUser) -> Result<RoomRow> {
    let id: Uuid = r.try_get("id")?;
    let owner_id: Uuid = r.try_get("owner_id")?;
    let role = member_role(&state.db, id, viewer.id).await?;
    let is_owner = owner_id == viewer.id;
    let can_moderate = is_owner || viewer.is_mod() || role.as_deref() == Some("moderator");
    Ok(RoomRow {
        view: CallRoomView {
            id,
            name: r.try_get("name")?,
            description: r.try_get("description")?,
            owner: user_from(r, "owner")?,
            visibility: RoomVisibility::parse(&r.try_get::<String, _>("visibility")?),
            persistent: r.try_get("persistent")?,
            created_at: r.try_get("created_at")?,
            last_active_at: r.try_get("last_active_at")?,
            participants: state.hub.topic_users(&Topic::Call(id)),
            can_edit: is_owner || viewer.is_mod(),
            can_moderate,
        },
    })
}

/// SQL condition: room `r` is visible for user $1 (with $2 = moderator).
const ROOM_VISIBLE: &str = "(r.visibility = 'members' OR r.owner_id = $1 OR $2 \
    OR EXISTS (SELECT 1 FROM call_room_members m WHERE m.room_id = r.id AND m.user_id = $1))";

async fn load_room(state: &AppState, user: &AuthUser, id: Uuid) -> Result<CallRoomView> {
    let row = sqlx::query(&format!("{} WHERE r.id = $3 AND {ROOM_VISIBLE}", room_select()))
        .bind(user.id)
        .bind(user.is_mod())
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(Error::NotFound("Raum"))?;
    Ok(room_from(state, &row, user).await?.view)
}

pub async fn list_rooms(state: &AppState, user: &AuthUser) -> Result<Vec<CallRoomView>> {
    let rows = sqlx::query(&format!("{} WHERE {ROOM_VISIBLE} ORDER BY r.last_active_at DESC LIMIT 200", room_select()))
        .bind(user.id)
        .bind(user.is_mod())
        .fetch_all(&state.db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(room_from(state, r, user).await?.view);
    }
    // Active rooms first.
    out.sort_by_key(|r| std::cmp::Reverse(r.participants.len()));
    Ok(out)
}

pub async fn overview(state: &AppState, user: &AuthUser) -> Result<CallsOverview> {
    let s = state.settings();
    Ok(CallsOverview {
        rooms: list_rooms(state, user).await?,
        history: history(&state.db, user, 30).await?,
        configured: state.config.livekit.is_some(),
        can_create: user.is_mod() || s.members_create_call_rooms,
    })
}

pub async fn room_detail(state: &AppState, user: &AuthUser, id: Uuid) -> Result<CallRoomDetail> {
    let room = load_room(state, user, id).await?;
    let rows = sqlx::query(&format!(
        "SELECT m.role, {} FROM call_room_members m JOIN users u ON u.id = m.user_id \
         WHERE m.room_id = $1 ORDER BY m.role DESC, lower(u.display_name)",
        user_cols("u", "u")
    ))
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let members = rows
        .iter()
        .map(|r| {
            Ok(RoomMember {
                user: user_from(r, "u")?,
                moderator: r.try_get::<String, _>("role")? == "moderator",
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let chat_channel_id = if state.settings().modules.chat {
        Some(crate::chat::room_channel(&state.db, id, &room.name).await?)
    } else {
        None
    };
    Ok(CallRoomDetail { room, members, chat_channel_id })
}

fn validate_room(input: &CallRoomInput) -> Result<(String, String)> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(Error::invalid("Der Raumname muss 1–80 Zeichen lang sein."));
    }
    let description = input.description.trim().to_string();
    if description.chars().count() > 1000 {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 1.000 Zeichen)."));
    }
    Ok((name, description))
}

pub async fn create_room(state: &AppState, user: &AuthUser, input: CallRoomInput) -> Result<CallRoomView> {
    if !state.settings().modules.calls {
        return Err(Error::invalid("Anrufe sind deaktiviert."));
    }
    if !user.is_mod() && !state.settings().members_create_call_rooms {
        return Err(Error::Forbidden);
    }
    state
        .limiter
        .hit(&format!("callroom:{}", user.id), 10, StdDuration::from_secs(3600))?;
    let (name, description) = validate_room(&input)?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO call_rooms (id, owner_id, name, description, visibility, persistent) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(user.id)
    .bind(&name)
    .bind(&description)
    .bind(input.visibility.as_str())
    .bind(input.persistent)
    .execute(&state.db)
    .await?;
    load_room(state, user, id).await
}

async fn editable_room(state: &AppState, user: &AuthUser, id: Uuid) -> Result<CallRoomView> {
    let room = load_room(state, user, id).await?;
    if !room.can_edit {
        return Err(Error::Forbidden);
    }
    Ok(room)
}

pub async fn update_room(state: &AppState, user: &AuthUser, id: Uuid, input: CallRoomInput) -> Result<CallRoomView> {
    editable_room(state, user, id).await?;
    let (name, description) = validate_room(&input)?;
    sqlx::query("UPDATE call_rooms SET name = $2, description = $3, visibility = $4, persistent = $5 WHERE id = $1")
        .bind(id)
        .bind(&name)
        .bind(&description)
        .bind(input.visibility.as_str())
        .bind(input.persistent)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE chat_channels SET name = $2 WHERE dm_key = $1")
        .bind(format!("room:{id}"))
        .bind(&name)
        .execute(&state.db)
        .await?;
    state.hub.publish(&Topic::Call(id), &emit(&CallEvent::RoomUpdated { room_id: id }));
    load_room(state, user, id).await
}

async fn remove_room(state: &AppState, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM call_rooms WHERE id = $1").bind(id).execute(&state.db).await?;
    sqlx::query("DELETE FROM chat_channels WHERE dm_key = $1")
        .bind(format!("room:{id}"))
        .execute(&state.db)
        .await?;
    state.hub.publish(&Topic::Call(id), &emit(&CallEvent::RoomDeleted { room_id: id }));
    Ok(())
}

pub async fn delete_room(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let room = editable_room(state, user, id).await?;
    remove_room(state, id).await?;
    if room.owner.id != user.id {
        crate::modlog::record(&state.db, Some(user.id), "call.room.delete", None, Some(room.owner.id), &room.name).await?;
    }
    Ok(())
}

/// Adds/updates (`Some(moderator)`) or removes (`None`) an invited member.
pub async fn set_member(state: &AppState, user: &AuthUser, room_id: Uuid, target: Uuid, role: Option<bool>) -> Result<()> {
    let room = load_room(state, user, room_id).await?;
    if !room.can_moderate {
        return Err(Error::Forbidden);
    }
    // Only owners/site moderators may appoint moderators.
    if role == Some(true) && !room.can_edit {
        return Err(Error::Forbidden);
    }
    match role {
        Some(moderator) => {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND is_active)")
                .bind(target)
                .fetch_one(&state.db)
                .await?;
            if !exists {
                return Err(Error::NotFound("Nutzer"));
            }
            sqlx::query(
                "INSERT INTO call_room_members (room_id, user_id, role) VALUES ($1, $2, $3) \
                 ON CONFLICT (room_id, user_id) DO UPDATE SET role = EXCLUDED.role",
            )
            .bind(room_id)
            .bind(target)
            .bind(if moderator { "moderator" } else { "member" })
            .execute(&state.db)
            .await?;
            if target != user.id {
                notifications::notify(
                    state,
                    NewNotification {
                        user_id: target,
                        kind: NotificationKind::Call,
                        actor_id: Some(user.id),
                        title: format!("{} hat dich in den Raum „{}“ eingeladen", user.display_name, room.name),
                        body: String::new(),
                        link: room.url(),
                        subject: None,
                    },
                )
                .await;
            }
        }
        None => {
            sqlx::query("DELETE FROM call_room_members WHERE room_id = $1 AND user_id = $2")
                .bind(room_id)
                .bind(target)
                .execute(&state.db)
                .await?;
        }
    }
    state.hub.publish(&Topic::Call(room_id), &emit(&CallEvent::RoomUpdated { room_id }));
    Ok(())
}

pub async fn join_room(state: &AppState, user: &AuthUser, id: Uuid) -> Result<JoinInfo> {
    let room = load_room(state, user, id).await?;
    state
        .limiter
        .hit(&format!("calljoin:{}", user.id), 30, StdDuration::from_secs(60))?;
    let info = join_info(state, user, room_lk_name(id), room.can_moderate)?;
    sqlx::query("UPDATE call_rooms SET last_active_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(info)
}

async fn moderated_room(state: &AppState, actor: &AuthUser, room_id: Uuid) -> Result<CallRoomView> {
    let room = load_room(state, actor, room_id).await?;
    if !room.can_moderate {
        return Err(Error::Forbidden);
    }
    Ok(room)
}

pub async fn mute_participant(state: &AppState, actor: &AuthUser, room_id: Uuid, target: Uuid) -> Result<()> {
    moderated_room(state, actor, room_id).await?;
    let cfg = require_configured(state)?;
    // Ask the client to mute itself too (works even if the API call fails).
    state.hub.send_user(target, &emit(&CallEvent::MuteRequest { room_id, by: actor.summary() }));
    livekit::mute_microphone(cfg, &room_lk_name(room_id), &target.to_string()).await?;
    Ok(())
}

pub async fn remove_participant(state: &AppState, actor: &AuthUser, room_id: Uuid, target: Uuid) -> Result<()> {
    let room = moderated_room(state, actor, room_id).await?;
    if target == room.owner.id && actor.id != room.owner.id && !actor.is_mod() {
        return Err(Error::invalid("Der Raumbesitzer kann nicht entfernt werden."));
    }
    let cfg = require_configured(state)?;
    state.hub.send_user(target, &emit(&CallEvent::Removed { room_id, by: actor.summary() }));
    let res = livekit::remove_participant(cfg, &room_lk_name(room_id), &target.to_string()).await;
    crate::modlog::record(
        &state.db,
        Some(actor.id),
        "call.participant.remove",
        None,
        Some(target),
        &room.name,
    )
    .await?;
    res
}

/// Publishes the live participant list of a room.
pub fn broadcast_participants(state: &AppState, room_id: Uuid) {
    let participants = state.hub.topic_users(&Topic::Call(room_id));
    state.hub.publish(
        &Topic::Call(room_id),
        &emit(&CallEvent::Participants { room_id, participants }),
    );
}

/// Whether `user` may subscribe to `Call(id)` (a room or a 1:1 call).
pub async fn can_subscribe(state: &AppState, user: Option<&AuthUser>, id: Uuid) -> bool {
    let Some(user) = user else { return false };
    if load_room(state, user, id).await.is_ok() {
        return true;
    }
    matches!(direct_call(&state.db, id).await, Ok(Some(c)) if c.caller.id == user.id || c.callee.id == user.id)
}

pub async fn on_join(state: &AppState, id: Uuid) {
    if is_room(&state.db, id).await {
        let _ = sqlx::query("UPDATE call_rooms SET last_active_at = now() WHERE id = $1")
            .bind(id)
            .execute(&state.db)
            .await;
        broadcast_participants(state, id);
    }
}

pub async fn on_leave(state: &AppState, id: Uuid) {
    if is_room(&state.db, id).await {
        let _ = sqlx::query("UPDATE call_rooms SET last_active_at = now() WHERE id = $1")
            .bind(id)
            .execute(&state.db)
            .await;
        broadcast_participants(state, id);
        return;
    }
    // A 1:1 call ends when nobody is connected to it anymore.
    if state.hub.topic_conn_count(&Topic::Call(id)) == 0 {
        if let Ok(Some(call)) = direct_call(&state.db, id).await {
            let next = match call.status {
                CallStatus::Ringing => Some(CallStatus::Cancelled),
                CallStatus::Accepted => Some(CallStatus::Ended),
                _ => None,
            };
            if let Some(status) = next {
                let _ = finish_call(state, call, status).await;
            }
        }
    }
}

async fn is_room(db: &PgPool, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM call_rooms WHERE id = $1)")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap_or(false)
}

/// Chat channel access for `room:<id>` channels of private rooms.
pub async fn room_channel_allowed(db: &PgPool, room_id: Uuid, user_id: Uuid) -> Option<bool> {
    let row: Option<(String, Uuid)> = sqlx::query_as("SELECT visibility, owner_id FROM call_rooms WHERE id = $1")
        .bind(room_id)
        .fetch_optional(db)
        .await
        .ok()?;
    let (visibility, owner) = row?;
    if visibility != "private" || owner == user_id {
        return Some(true);
    }
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM call_room_members WHERE room_id = $1 AND user_id = $2) \
         OR EXISTS (SELECT 1 FROM users WHERE id = $2 AND role IN ('moderator', 'admin'))",
    )
    .bind(room_id)
    .bind(user_id)
    .fetch_one(db)
    .await
    .unwrap_or(false);
    Some(allowed)
}

/// Removes spontaneous rooms that have been empty for a while.
pub async fn cleanup_rooms(state: &AppState) -> Result<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM call_rooms WHERE NOT persistent \
         AND last_active_at < now() - make_interval(mins => $1)",
    )
    .bind(SPONTANEOUS_IDLE_MINUTES)
    .fetch_all(&state.db)
    .await?;
    for id in ids {
        if state.hub.topic_conn_count(&Topic::Call(id)) == 0 {
            remove_room(state, id).await?;
        }
    }
    // Calls whose ring timer was lost (server restart).
    let stale: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM direct_calls WHERE status = 'ringing' AND created_at < now() - make_interval(secs => $1)",
    )
    .bind((RING_SECONDS + 30) as f64)
    .fetch_all(&state.db)
    .await?;
    for id in stale {
        if let Some(call) = direct_call(&state.db, id).await? {
            finish_call(state, call, CallStatus::Missed).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 1:1 calls
// ---------------------------------------------------------------------------

fn call_select() -> String {
    format!(
        "SELECT c.id, c.video, c.status, c.created_at, c.answered_at, c.ended_at, {}, {} \
         FROM direct_calls c JOIN users a ON a.id = c.caller_id JOIN users b ON b.id = c.callee_id ",
        user_cols("a", "caller"),
        user_cols("b", "callee")
    )
}

fn call_from(r: &PgRow) -> Result<DirectCallView, sqlx::Error> {
    Ok(DirectCallView {
        id: r.try_get("id")?,
        caller: user_from(r, "caller")?,
        callee: user_from(r, "callee")?,
        video: r.try_get("video")?,
        status: CallStatus::parse(&r.try_get::<String, _>("status")?),
        created_at: r.try_get("created_at")?,
        answered_at: r.try_get("answered_at")?,
        ended_at: r.try_get("ended_at")?,
    })
}

async fn direct_call(db: &PgPool, id: Uuid) -> Result<Option<DirectCallView>> {
    let row = sqlx::query(&format!("{} WHERE c.id = $1", call_select()))
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| call_from(&r)).transpose()?)
}

pub async fn history(db: &PgPool, user: &AuthUser, limit: i64) -> Result<Vec<DirectCallView>> {
    let rows = sqlx::query(&format!(
        "{} WHERE c.caller_id = $1 OR c.callee_id = $1 ORDER BY c.created_at DESC LIMIT $2",
        call_select()
    ))
    .bind(user.id)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(call_from).collect::<Result<_, _>>()?)
}

/// Calls currently ringing for the user (e.g. after a page reload).
pub async fn incoming(db: &PgPool, user: &AuthUser) -> Result<Vec<DirectCallView>> {
    let rows = sqlx::query(&format!(
        "{} WHERE c.callee_id = $1 AND c.status = 'ringing' \
         AND c.created_at > now() - make_interval(secs => $2) ORDER BY c.created_at DESC",
        call_select()
    ))
    .bind(user.id)
    .bind(RING_SECONDS as f64)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(call_from).collect::<Result<_, _>>()?)
}

fn notify_status(state: &AppState, call: &DirectCallView) {
    let msg = emit(&CallEvent::Status { call: call.clone() });
    state.hub.send_user(call.caller.id, &msg);
    state.hub.send_user(call.callee.id, &msg);
}

async fn finish_call(state: &AppState, call: DirectCallView, status: CallStatus) -> Result<DirectCallView> {
    let updated = sqlx::query(
        "UPDATE direct_calls SET status = $2, ended_at = now() WHERE id = $1 AND status IN ('ringing', 'accepted')",
    )
    .bind(call.id)
    .bind(status.as_str())
    .execute(&state.db)
    .await?;
    let call = direct_call(&state.db, call.id).await?.ok_or(Error::NotFound("Anruf"))?;
    if updated.rows_affected() > 0 {
        notify_status(state, &call);
        if matches!(status, CallStatus::Missed | CallStatus::Cancelled) {
            notifications::notify(
                state,
                NewNotification {
                    user_id: call.callee.id,
                    kind: NotificationKind::Call,
                    actor_id: Some(call.caller.id),
                    title: format!("Verpasster Anruf von {}", call.caller.display_name),
                    body: String::new(),
                    link: "/calls".into(),
                    subject: None,
                },
            )
            .await;
        }
    }
    Ok(call)
}

pub async fn start_call(state: &AppState, caller: &AuthUser, callee_id: Uuid, video: bool) -> Result<DirectCallDetail> {
    require_configured(state)?;
    if callee_id == caller.id {
        return Err(Error::invalid("Du kannst dich nicht selbst anrufen."));
    }
    state
        .limiter
        .hit(&format!("call:{}", caller.id), 10, StdDuration::from_secs(60))?;
    let callee_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND is_active AND (banned_until IS NULL OR banned_until < now()))",
    )
    .bind(callee_id)
    .fetch_one(&state.db)
    .await?;
    if !callee_ok {
        return Err(Error::NotFound("Nutzer"));
    }
    // Re-use an open call between the two.
    let open: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM direct_calls WHERE status IN ('ringing', 'accepted') \
         AND ((caller_id = $1 AND callee_id = $2) OR (caller_id = $2 AND callee_id = $1)) \
         AND created_at > now() - interval '12 hours' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(caller.id)
    .bind(callee_id)
    .fetch_optional(&state.db)
    .await?;
    if let Some(id) = open {
        return direct_detail(state, caller, id).await;
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO direct_calls (id, caller_id, callee_id, video) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(caller.id)
        .bind(callee_id)
        .bind(video)
        .execute(&state.db)
        .await?;
    let call = direct_call(&state.db, id).await?.ok_or(Error::NotFound("Anruf"))?;
    if state.hub.is_online(callee_id) {
        state.hub.send_user(callee_id, &emit(&CallEvent::Incoming { call: call.clone() }));
        let st = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(StdDuration::from_secs(RING_SECONDS as u64)).await;
            if let Ok(Some(c)) = direct_call(&st.db, id).await {
                if c.status == CallStatus::Ringing {
                    let _ = finish_call(&st, c, CallStatus::Missed).await;
                }
            }
        });
    } else {
        finish_call(state, call, CallStatus::Missed).await?;
    }
    direct_detail(state, caller, id).await
}

pub async fn direct_detail(state: &AppState, user: &AuthUser, id: Uuid) -> Result<DirectCallDetail> {
    let call = direct_call(&state.db, id).await?.ok_or(Error::NotFound("Anruf"))?;
    if call.caller.id != user.id && call.callee.id != user.id {
        return Err(Error::NotFound("Anruf"));
    }
    // The callee only gets a token after accepting.
    let may_join = match call.status {
        CallStatus::Accepted => true,
        CallStatus::Ringing => call.caller.id == user.id,
        _ => false,
    };
    let join = if may_join && state.config.livekit.is_some() {
        Some(join_info(state, user, direct_lk_name(id), false)?)
    } else {
        None
    };
    Ok(DirectCallDetail { call, join })
}

pub async fn answer(state: &AppState, user: &AuthUser, id: Uuid, accept: bool) -> Result<DirectCallDetail> {
    let call = direct_call(&state.db, id).await?.ok_or(Error::NotFound("Anruf"))?;
    if call.callee.id != user.id {
        return Err(Error::NotFound("Anruf"));
    }
    if call.status != CallStatus::Ringing {
        return Err(Error::invalid(format!("Dieser Anruf ist nicht mehr aktiv ({}).", call.status.label())));
    }
    if accept {
        sqlx::query("UPDATE direct_calls SET status = 'accepted', answered_at = now() WHERE id = $1 AND status = 'ringing'")
            .bind(id)
            .execute(&state.db)
            .await?;
        let call = direct_call(&state.db, id).await?.ok_or(Error::NotFound("Anruf"))?;
        notify_status(state, &call);
    } else {
        finish_call(state, call, CallStatus::Declined).await?;
    }
    direct_detail(state, user, id).await
}

pub async fn hangup(state: &AppState, user: &AuthUser, id: Uuid) -> Result<DirectCallView> {
    let call = direct_call(&state.db, id).await?.ok_or(Error::NotFound("Anruf"))?;
    if call.caller.id != user.id && call.callee.id != user.id {
        return Err(Error::NotFound("Anruf"));
    }
    let status = match call.status {
        CallStatus::Ringing if call.caller.id == user.id => CallStatus::Cancelled,
        CallStatus::Ringing => CallStatus::Declined,
        CallStatus::Accepted => CallStatus::Ended,
        _ => return Ok(call),
    };
    finish_call(state, call, status).await
}
