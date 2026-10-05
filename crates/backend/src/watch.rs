//! Watch together: persistence and permissions (rooms, roles, chat,
//! moderation, requests, emotes, shows, playlist). The live playback engine
//! is in `watch_live`, the media library in `watch_media`.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use shared::watch::{
    Emote, EventInfo, EventInput, EventSaved, EventStatus, ModerationKind, PlaylistItem, PlaylistMode,
    ProposalInfo, ProposalInput, ProposalStatus, RestrictedUser, RoomRole, RoomRoleRow, ShowItem, ShowPhase,
    SourceKind, WatchChatMessage, WatchRoomInput, WatchRoomView, WatchVisibility, CHAT_MAX_CHARS,
    OPEN_PROPOSALS_PER_USER,
};
use shared::AuthUser;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::users::{user_cols, user_from};
use crate::watch_live;

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RoomRow {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub owner_id: Uuid,
    pub visibility: WatchVisibility,
    pub playlist_mode: PlaylistMode,
    pub slow_mode_ms: i64,
    pub voteskip_percent: i32,
}

const ROOM_COLS: &str =
    "r.id, r.slug, r.name, r.description, r.owner_id, r.visibility, r.playlist_mode, r.slow_mode_ms, r.voteskip_percent";

fn room_row_from(r: &PgRow) -> Result<RoomRow, sqlx::Error> {
    Ok(RoomRow {
        id: r.try_get("id")?,
        slug: r.try_get("slug")?,
        name: r.try_get("name")?,
        description: r.try_get("description")?,
        owner_id: r.try_get("owner_id")?,
        visibility: WatchVisibility::parse(&r.try_get::<String, _>("visibility")?),
        playlist_mode: PlaylistMode::parse(&r.try_get::<String, _>("playlist_mode")?),
        slow_mode_ms: r.try_get("slow_mode_ms")?,
        voteskip_percent: r.try_get("voteskip_percent")?,
    })
}

pub async fn room_row(db: &PgPool, id: Uuid) -> Result<Option<RoomRow>> {
    let row = sqlx::query(&format!("SELECT {ROOM_COLS} FROM watch_rooms r WHERE r.id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| room_row_from(&r)).transpose()?)
}

pub async fn room_by_slug(db: &PgPool, slug: &str) -> Result<Option<RoomRow>> {
    let row = sqlx::query(&format!("SELECT {ROOM_COLS} FROM watch_rooms r WHERE r.slug = $1"))
        .bind(slug.to_lowercase())
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| room_row_from(&r)).transpose()?)
}

/// The user's role in a room (owner, mod, member or none).
pub async fn role_of(db: &PgPool, room: &RoomRow, user_id: Uuid) -> Result<Option<RoomRole>> {
    if room.owner_id == user_id {
        return Ok(Some(RoomRole::Owner));
    }
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM watch_room_roles WHERE room_id = $1 AND user_id = $2")
        .bind(room.id)
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    Ok(role.and_then(|r| RoomRole::parse(&r)))
}

pub fn can_access(room: &RoomRow, user: &AuthUser, role: Option<RoomRole>) -> bool {
    room.visibility == WatchVisibility::Members || role.is_some() || user.is_mod()
}

/// Owner, room moderators and site moderators drive playback and moderate.
pub fn can_control(user: &AuthUser, role: Option<RoomRole>) -> bool {
    matches!(role, Some(RoomRole::Owner | RoomRole::Mod)) || user.is_mod()
}

pub fn can_add(room: &RoomRow, user: &AuthUser, role: Option<RoomRole>, leader: bool) -> bool {
    if can_control(user, role) || leader {
        return true;
    }
    match room.playlist_mode {
        PlaylistMode::Open => true,
        PlaylistMode::Members => role.is_some(),
        PlaylistMode::Mods => false,
    }
}

/// Loads a room the user may enter, with their role.
pub async fn accessible(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<(RoomRow, Option<RoomRole>)> {
    let room = room_row(db, id).await?.ok_or(Error::NotFound("Raum"))?;
    let role = role_of(db, &room, user.id).await?;
    if !can_access(&room, user, role) {
        return Err(Error::NotFound("Raum"));
    }
    if effective_for(db, room.id, user.id).await?.banned && !user.is_mod() {
        return Err(Error::invalid("Du bist aus diesem Raum verbannt."));
    }
    Ok((room, role))
}

async fn controlled(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<(RoomRow, Option<RoomRole>)> {
    let (room, role) = accessible(db, user, id).await?;
    if !can_control(user, role) {
        return Err(Error::Forbidden);
    }
    Ok((room, role))
}

async fn view_of(state: &AppState, room: &RoomRow, user: &AuthUser) -> Result<WatchRoomView> {
    let role = role_of(&state.db, room, user.id).await?;
    let owner = crate::users::summary(&state.db, room.owner_id)
        .await?
        .ok_or(Error::NotFound("Nutzer"))?;
    let next: Option<(String, i64)> = sqlx::query_as(
        "SELECT title, starts_at FROM watch_events WHERE room_id = $1 AND status = 'scheduled' \
         ORDER BY starts_at LIMIT 1",
    )
    .bind(room.id)
    .fetch_optional(&state.db)
    .await?;
    let (now_playing, show) = watch_live::summary(state, room.id);
    Ok(WatchRoomView {
        id: room.id,
        slug: room.slug.clone(),
        name: room.name.clone(),
        description: room.description.clone(),
        owner,
        visibility: room.visibility,
        playlist_mode: room.playlist_mode,
        slow_mode_ms: room.slow_mode_ms,
        voteskip_percent: room.voteskip_percent,
        viewers: state.hub.topic_users(&shared::ws::Topic::Watch(room.id)).len() as i64,
        now_playing,
        show,
        next_event: next.map(|(title, starts_at_ms)| shared::watch::NextEvent { title, starts_at_ms }),
        can_edit: role == Some(RoomRole::Owner) || user.is_admin(),
        can_control: can_control(user, role),
    })
}

pub async fn room_view(state: &AppState, user: &AuthUser, id: Uuid) -> Result<WatchRoomView> {
    let (room, _) = accessible(&state.db, user, id).await?;
    view_of(state, &room, user).await
}

pub async fn room_view_by_slug(state: &AppState, user: &AuthUser, slug: &str) -> Result<WatchRoomView> {
    let room = room_by_slug(&state.db, slug).await?.ok_or(Error::NotFound("Raum"))?;
    room_view(state, user, room.id).await
}

/// Live rooms first (busiest on top), then the soonest upcoming show.
pub async fn list_rooms(state: &AppState, user: &AuthUser) -> Result<Vec<WatchRoomView>> {
    let rows = sqlx::query(&format!(
        "SELECT {ROOM_COLS} FROM watch_rooms r WHERE r.visibility = 'members' OR r.owner_id = $1 OR $2 \
         OR EXISTS (SELECT 1 FROM watch_room_roles x WHERE x.room_id = r.id AND x.user_id = $1) \
         ORDER BY r.last_active_at DESC LIMIT 200"
    ))
    .bind(user.id)
    .bind(user.is_mod())
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(view_of(state, &room_row_from(r)?, user).await?);
    }
    let rank = |r: &WatchRoomView| {
        if r.show.as_ref().is_some_and(|s| s.live) {
            0
        } else if r.viewers > 0 || r.now_playing.is_some() {
            1
        } else if r.next_event.is_some() {
            2
        } else {
            3
        }
    };
    out.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then_with(|| b.viewers.cmp(&a.viewers)).then_with(|| {
            let sa = a.next_event.as_ref().map(|n| n.starts_at_ms).unwrap_or(i64::MAX);
            let sb = b.next_event.as_ref().map(|n| n.starts_at_ms).unwrap_or(i64::MAX);
            sa.cmp(&sb)
        })
    });
    Ok(out)
}

fn validate_room(input: &WatchRoomInput) -> Result<(String, String)> {
    let name = input.name.trim().to_string();
    if name.chars().count() < 2 || name.chars().count() > 60 {
        return Err(Error::invalid("Der Raumname muss 2–60 Zeichen lang sein."));
    }
    let description = input.description.trim().to_string();
    if description.chars().count() > 1000 {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 1.000 Zeichen)."));
    }
    if !(1..=100).contains(&input.voteskip_percent) {
        return Err(Error::invalid("Die Voteskip-Schwelle muss zwischen 1 und 100 % liegen."));
    }
    Ok((name, description))
}

async fn unique_slug(db: &PgPool, name: &str) -> Result<String> {
    let mut base = shared::util::slug_base(name);
    if base.is_empty() {
        base = "raum".into();
    }
    base.truncate(40);
    let base = base.trim_end_matches('-').to_string();
    for n in 1..100 {
        let candidate = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watch_rooms WHERE slug = $1)")
            .bind(&candidate)
            .fetch_one(db)
            .await?;
        if !taken {
            return Ok(candidate);
        }
    }
    Ok(format!("{base}-{}", &Uuid::now_v7().simple().to_string()[26..]))
}

pub async fn create_room(state: &AppState, user: &AuthUser, input: WatchRoomInput) -> Result<WatchRoomView> {
    if !user.is_mod() && !state.settings().members_create_watch_rooms {
        return Err(Error::Forbidden);
    }
    state
        .limiter
        .hit(&format!("watchroom:{}", user.id), 10, StdDuration::from_secs(3600))?;
    let (name, description) = validate_room(&input)?;
    let id = Uuid::now_v7();
    let slug = unique_slug(&state.db, &name).await?;
    sqlx::query(
        "INSERT INTO watch_rooms (id, slug, name, description, owner_id, visibility, playlist_mode, voteskip_percent) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(&slug)
    .bind(&name)
    .bind(&description)
    .bind(user.id)
    .bind(input.visibility.as_str())
    .bind(input.playlist_mode.as_str())
    .bind(input.voteskip_percent)
    .execute(&state.db)
    .await?;
    room_view(state, user, id).await
}

pub async fn update_room(state: &AppState, user: &AuthUser, id: Uuid, input: WatchRoomInput) -> Result<WatchRoomView> {
    let (room, role) = accessible(&state.db, user, id).await?;
    if role != Some(RoomRole::Owner) && !user.is_admin() {
        return Err(Error::Forbidden);
    }
    let (name, description) = validate_room(&input)?;
    sqlx::query(
        "UPDATE watch_rooms SET name = $2, description = $3, visibility = $4, playlist_mode = $5, voteskip_percent = $6 \
         WHERE id = $1",
    )
    .bind(room.id)
    .bind(&name)
    .bind(&description)
    .bind(input.visibility.as_str())
    .bind(input.playlist_mode.as_str())
    .bind(input.voteskip_percent)
    .execute(&state.db)
    .await?;
    watch_live::room_settings_changed(state, room.id).await;
    room_view(state, user, id).await
}

pub async fn delete_room(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let (room, role) = accessible(&state.db, user, id).await?;
    if role != Some(RoomRole::Owner) && !user.is_admin() {
        return Err(Error::Forbidden);
    }
    let live: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watch_events WHERE room_id = $1 AND status = 'live')")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    if live {
        return Err(Error::invalid("In diesem Raum läuft gerade eine Vorstellung."));
    }
    watch_live::close_room(state, id, "Der Raum wurde gelöscht.").await;
    sqlx::query("DELETE FROM watch_rooms WHERE id = $1").bind(id).execute(&state.db).await?;
    if room.owner_id != user.id {
        crate::modlog::record(&state.db, Some(user.id), "watch.room.delete", None, Some(room.owner_id), &room.name).await?;
    }
    Ok(())
}

pub async fn set_slow_mode(state: &AppState, user: &AuthUser, id: Uuid, ms: i64) -> Result<()> {
    let (room, _) = controlled(&state.db, user, id).await?;
    if !(0..=120_000).contains(&ms) {
        return Err(Error::invalid("Slow-Mode: 0 bis 120 Sekunden."));
    }
    sqlx::query("UPDATE watch_rooms SET slow_mode_ms = $2 WHERE id = $1")
        .bind(room.id)
        .bind(ms)
        .execute(&state.db)
        .await?;
    watch_live::apply_slow_mode(state, room.id, ms).await;
    Ok(())
}

pub async fn roles(state: &AppState, user: &AuthUser, id: Uuid) -> Result<Vec<RoomRoleRow>> {
    let (room, _) = controlled(&state.db, user, id).await?;
    let rows = sqlx::query(&format!(
        "SELECT x.role, {} FROM watch_room_roles x JOIN users u ON u.id = x.user_id WHERE x.room_id = $1 \
         ORDER BY x.role, lower(u.display_name)",
        user_cols("u", "u")
    ))
    .bind(room.id)
    .fetch_all(&state.db)
    .await?;
    rows.iter()
        .filter_map(|r| {
            let role = RoomRole::parse(&r.try_get::<String, _>("role").ok()?)?;
            Some(user_from(r, "u").map(|user| RoomRoleRow { user, role }).map_err(Error::from))
        })
        .collect()
}

/// Owner/admin: grants `mod` / `member` or removes (`None`) a room role.
pub async fn set_role(state: &AppState, user: &AuthUser, id: Uuid, target: Uuid, role: Option<RoomRole>) -> Result<()> {
    let (room, my_role) = accessible(&state.db, user, id).await?;
    let is_owner = my_role == Some(RoomRole::Owner) || user.is_admin();
    // Mods may invite members but not appoint mods.
    let allowed = match role {
        Some(RoomRole::Mod) | None => is_owner || (role.is_none() && can_control(user, my_role)),
        Some(RoomRole::Member) => can_control(user, my_role),
        Some(RoomRole::Owner) => false,
    };
    if !allowed {
        return Err(Error::Forbidden);
    }
    if target == room.owner_id {
        return Err(Error::invalid("Das ist der Inhaber des Raums."));
    }
    match role {
        Some(r) => {
            sqlx::query(
                "INSERT INTO watch_room_roles (room_id, user_id, role) VALUES ($1, $2, $3) \
                 ON CONFLICT (room_id, user_id) DO UPDATE SET role = EXCLUDED.role",
            )
            .bind(room.id)
            .bind(target)
            .bind(r.as_str())
            .execute(&state.db)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM watch_room_roles WHERE room_id = $1 AND user_id = $2")
                .bind(room.id)
                .bind(target)
                .execute(&state.db)
                .await?;
        }
    }
    watch_live::refresh_user(state, room.id, target).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

pub async fn insert_chat(db: &PgPool, room_id: Uuid, user: &AuthUser, body: &str) -> Result<WatchChatMessage> {
    let text: String = body.trim().chars().take(CHAT_MAX_CHARS).collect();
    let id = Uuid::now_v7();
    let at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO watch_chat (id, room_id, user_id, body) VALUES ($1, $2, $3, $4) RETURNING created_at",
    )
    .bind(id)
    .bind(room_id)
    .bind(user.id)
    .bind(&text)
    .fetch_one(db)
    .await?;
    Ok(WatchChatMessage { id, user: user.summary(), body: text, at_ms: at.timestamp_millis() })
}

pub async fn chat_history(db: &PgPool, room_id: Uuid, limit: i64) -> Result<Vec<WatchChatMessage>> {
    let rows = sqlx::query(&format!(
        "SELECT c.id, c.body, c.created_at, {} FROM watch_chat c JOIN users u ON u.id = c.user_id \
         WHERE c.room_id = $1 AND c.deleted_at IS NULL ORDER BY c.created_at DESC LIMIT $2",
        user_cols("u", "u")
    ))
    .bind(room_id)
    .bind(limit)
    .fetch_all(db)
    .await?;
    let mut out = rows
        .iter()
        .map(|r| {
            Ok(WatchChatMessage {
                id: r.try_get("id")?,
                user: user_from(r, "u")?,
                body: r.try_get("body")?,
                at_ms: r.try_get::<DateTime<Utc>, _>("created_at")?.timestamp_millis(),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    out.reverse();
    Ok(out)
}

pub async fn delete_chat(state: &AppState, user: &AuthUser, room_id: Uuid, message_id: Uuid) -> Result<()> {
    let (room, _) = controlled(&state.db, user, room_id).await?;
    let author: Option<Uuid> = sqlx::query_scalar(
        "UPDATE watch_chat SET deleted_at = now() WHERE id = $1 AND room_id = $2 AND deleted_at IS NULL RETURNING user_id",
    )
    .bind(message_id)
    .bind(room.id)
    .fetch_optional(&state.db)
    .await?;
    if let Some(target) = author {
        record_moderation(&state.db, room.id, target, user.id, "delete_msg", None, None).await?;
    }
    watch_live::chat_removed(state, room.id, vec![message_id]);
    Ok(())
}

// ---------------------------------------------------------------------------
// Moderation (room-level timeout / ban)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct ModEffect {
    pub banned: bool,
    pub muted_until_ms: Option<i64>,
}

pub async fn effective_for(db: &PgPool, room_id: Uuid, user_id: Uuid) -> Result<ModEffect> {
    let row: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT kind, until_at FROM watch_moderation WHERE room_id = $1 AND target_user_id = $2 \
         AND kind IN ('timeout', 'ban', 'unban') ORDER BY created_at DESC LIMIT 1",
    )
    .bind(room_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(match row {
        Some((kind, _)) if kind == "ban" => ModEffect { banned: true, muted_until_ms: None },
        Some((kind, until)) if kind == "timeout" => ModEffect {
            banned: false,
            muted_until_ms: until.map(|u| u.timestamp_millis()),
        },
        _ => ModEffect::default(),
    })
}

async fn record_moderation(
    db: &PgPool,
    room_id: Uuid,
    target: Uuid,
    actor: Uuid,
    kind: &str,
    until: Option<DateTime<Utc>>,
    reason: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO watch_moderation (id, room_id, target_user_id, actor_user_id, kind, until_at, reason) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(room_id)
    .bind(target)
    .bind(actor)
    .bind(kind)
    .bind(until)
    .bind(reason)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn moderate(
    state: &AppState,
    user: &AuthUser,
    room_id: Uuid,
    target: Uuid,
    kind: ModerationKind,
    minutes: Option<i64>,
    reason: &str,
) -> Result<()> {
    let (room, my_role) = controlled(&state.db, user, room_id).await?;
    state
        .limiter
        .hit(&format!("watchmod:{}", user.id), 30, StdDuration::from_secs(60))?;
    if target == user.id {
        return Err(Error::invalid("Du kannst dich nicht selbst moderieren."));
    }
    let target_user = crate::users::find_auth_user(&state.db, target)
        .await?
        .ok_or(Error::NotFound("Nutzer"))?;
    let target_role = role_of(&state.db, &room, target).await?;
    if kind != ModerationKind::Unban {
        if target_role == Some(RoomRole::Owner) {
            return Err(Error::invalid("Der Inhaber kann nicht moderiert werden."));
        }
        // Room mods reach regular users only; owners/site mods reach mods too.
        let i_am_mod_only = my_role == Some(RoomRole::Mod) && !user.is_mod();
        if i_am_mod_only && (target_role == Some(RoomRole::Mod) || target_user.is_mod()) {
            return Err(Error::invalid("Moderatoren können keine anderen Moderatoren oder Admins moderieren."));
        }
    }
    let reason = reason.trim();
    let reason = (!reason.is_empty()).then(|| reason.chars().take(200).collect::<String>());
    let (kind_str, until) = match kind {
        ModerationKind::Timeout => {
            let mins = minutes.unwrap_or(5).clamp(1, 1440);
            ("timeout", Some(Utc::now() + chrono::Duration::minutes(mins)))
        }
        ModerationKind::Ban => ("ban", None),
        ModerationKind::Unban => ("unban", None),
    };
    record_moderation(&state.db, room.id, target, user.id, kind_str, until, reason.as_deref()).await?;
    let effect = effective_for(&state.db, room.id, target).await?;
    watch_live::apply_moderation(state, room.id, target, effect).await;
    Ok(())
}

pub async fn restricted(state: &AppState, user: &AuthUser, room_id: Uuid) -> Result<Vec<RestrictedUser>> {
    let (room, _) = controlled(&state.db, user, room_id).await?;
    let rows = sqlx::query(&format!(
        "SELECT DISTINCT ON (m.target_user_id) m.kind, m.until_at, m.reason, {} \
         FROM watch_moderation m JOIN users u ON u.id = m.target_user_id \
         WHERE m.room_id = $1 AND m.kind IN ('timeout', 'ban', 'unban') \
         ORDER BY m.target_user_id, m.created_at DESC",
        user_cols("u", "u")
    ))
    .bind(room.id)
    .fetch_all(&state.db)
    .await?;
    let now = Utc::now();
    let mut out = Vec::new();
    for r in &rows {
        let kind: String = r.try_get("kind")?;
        let until: Option<DateTime<Utc>> = r.try_get("until_at")?;
        let active = kind == "ban" || (kind == "timeout" && until.is_some_and(|u| u > now));
        if active {
            out.push(RestrictedUser {
                user: user_from(r, "u")?,
                banned: kind == "ban",
                until_ms: until.map(|u| u.timestamp_millis()),
                reason: r.try_get("reason")?,
            });
        }
    }
    out.sort_by_key(|a| a.user.display_name.to_lowercase());
    Ok(out)
}

// ---------------------------------------------------------------------------
// Requests (proposals) with voting
// ---------------------------------------------------------------------------

pub async fn proposals(state: &AppState, user: &AuthUser, room_id: Uuid, all: bool) -> Result<Vec<ProposalInfo>> {
    let (room, role) = accessible(&state.db, user, room_id).await?;
    let include_all = all && can_control(user, role);
    let rows = sqlx::query(&format!(
        "SELECT p.id, p.title, p.notes, p.url, p.status, p.created_at, \
           (SELECT count(*) FROM watch_votes v WHERE v.proposal_id = p.id) AS votes, \
           EXISTS (SELECT 1 FROM watch_votes v WHERE v.proposal_id = p.id AND v.user_id = $2) AS mine, {} \
         FROM watch_proposals p JOIN users u ON u.id = p.created_by \
         WHERE p.room_id = $1 AND ($3 OR p.status = 'open') \
         ORDER BY votes DESC, p.created_at",
        user_cols("u", "u")
    ))
    .bind(room.id)
    .bind(user.id)
    .bind(include_all)
    .fetch_all(&state.db)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(ProposalInfo {
                id: r.try_get("id")?,
                title: r.try_get("title")?,
                notes: r.try_get("notes")?,
                url: r.try_get("url")?,
                status: ProposalStatus::parse(&r.try_get::<String, _>("status")?),
                votes: r.try_get("votes")?,
                my_vote: r.try_get("mine")?,
                created_by: user_from(r, "u")?,
                created_at_ms: r.try_get::<DateTime<Utc>, _>("created_at")?.timestamp_millis(),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Error::from)
}

pub async fn create_proposal(state: &AppState, user: &AuthUser, room_id: Uuid, input: ProposalInput) -> Result<()> {
    let (room, _) = accessible(&state.db, user, room_id).await?;
    state
        .limiter
        .hit(&format!("watchprop:{}", user.id), 10, StdDuration::from_secs(60))?;
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(Error::invalid("Der Titel muss 1–120 Zeichen lang sein."));
    }
    let notes = input.notes.trim();
    if notes.chars().count() > 500 {
        return Err(Error::invalid("Die Notiz ist zu lang (max. 500 Zeichen)."));
    }
    let url = input.url.trim();
    if !url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 300 {
        return Err(Error::invalid("Bitte gib einen gültigen Link an (http/https)."));
    }
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM watch_proposals WHERE room_id = $1 AND created_by = $2 AND status = 'open'",
    )
    .bind(room.id)
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if open >= OPEN_PROPOSALS_PER_USER {
        return Err(Error::invalid(format!(
            "Du hast schon {OPEN_PROPOSALS_PER_USER} offene Wünsche in diesem Raum."
        )));
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO watch_proposals (id, room_id, title, notes, url, created_by) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(id)
        .bind(room.id)
        .bind(title)
        .bind((!notes.is_empty()).then_some(notes))
        .bind((!url.is_empty()).then_some(url))
        .bind(user.id)
        .execute(&state.db)
        .await?;
    // Proposers support their own request.
    sqlx::query("INSERT INTO watch_votes (proposal_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    watch_live::proposals_changed(state, room.id);
    Ok(())
}

async fn proposal_room(db: &PgPool, id: Uuid) -> Result<(Uuid, Uuid, ProposalStatus)> {
    let row: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("SELECT room_id, created_by, status FROM watch_proposals WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?;
    let (room, author, status) = row.ok_or(Error::NotFound("Wunsch"))?;
    Ok((room, author, ProposalStatus::parse(&status)))
}

pub async fn toggle_vote(state: &AppState, user: &AuthUser, proposal_id: Uuid) -> Result<()> {
    let (room_id, _, status) = proposal_room(&state.db, proposal_id).await?;
    accessible(&state.db, user, room_id).await?;
    if status != ProposalStatus::Open {
        return Err(Error::invalid("Für diesen Wunsch kann nicht mehr abgestimmt werden."));
    }
    let removed = sqlx::query("DELETE FROM watch_votes WHERE proposal_id = $1 AND user_id = $2")
        .bind(proposal_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    if removed.rows_affected() == 0 {
        sqlx::query("INSERT INTO watch_votes (proposal_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(proposal_id)
            .bind(user.id)
            .execute(&state.db)
            .await?;
    }
    watch_live::proposals_changed(state, room_id);
    Ok(())
}

pub async fn delete_proposal(state: &AppState, user: &AuthUser, proposal_id: Uuid) -> Result<()> {
    let (room_id, author, status) = proposal_room(&state.db, proposal_id).await?;
    let (_, role) = accessible(&state.db, user, room_id).await?;
    let own_open = author == user.id && status == ProposalStatus::Open;
    if !own_open && !can_control(user, role) {
        return Err(Error::Forbidden);
    }
    sqlx::query("DELETE FROM watch_proposals WHERE id = $1").bind(proposal_id).execute(&state.db).await?;
    watch_live::proposals_changed(state, room_id);
    Ok(())
}

pub async fn set_proposal_status(state: &AppState, user: &AuthUser, proposal_id: Uuid, status: ProposalStatus) -> Result<()> {
    let (room_id, _, _) = proposal_room(&state.db, proposal_id).await?;
    controlled(&state.db, user, room_id).await?;
    sqlx::query("UPDATE watch_proposals SET status = $2 WHERE id = $1")
        .bind(proposal_id)
        .bind(status.as_str())
        .execute(&state.db)
        .await?;
    watch_live::proposals_changed(state, room_id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Emotes
// ---------------------------------------------------------------------------

pub async fn emotes(db: &PgPool) -> Result<Vec<Emote>> {
    let rows: Vec<(Uuid, String, Uuid)> = sqlx::query_as("SELECT id, code, file_id FROM watch_emotes ORDER BY code")
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, code, file_id)| Emote { id, code, url: format!("/files/{file_id}") })
        .collect())
}

pub async fn create_emote(state: &AppState, user: &AuthUser, code: &str, file_id: Uuid) -> Result<Emote> {
    if !user.is_admin() {
        return Err(Error::Forbidden);
    }
    let code = code.trim().to_lowercase();
    if code.len() < 2 || code.len() > 24 || !code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(Error::invalid("Der Code muss 2–24 Zeichen aus a–z, 0–9 und _ haben."));
    }
    let rec = crate::files::get_record(&state.db, file_id).await?.ok_or(Error::NotFound("Datei"))?;
    if rec.owner_id != user.id {
        return Err(Error::Forbidden);
    }
    if !matches!(rec.view.mime_type.as_str(), "image/png" | "image/gif" | "image/webp") {
        return Err(Error::invalid("Emotes müssen PNG, GIF oder WebP sein."));
    }
    if rec.view.size_bytes > 256 * 1024 {
        return Err(Error::invalid("Das Bild ist zu groß (max. 256 KB)."));
    }
    // Emotes are shown to everyone.
    sqlx::query("UPDATE files SET visibility = 'public', purpose = 'watch' WHERE id = $1")
        .bind(file_id)
        .execute(&state.db)
        .await?;
    let id = Uuid::now_v7();
    let res = sqlx::query("INSERT INTO watch_emotes (id, code, file_id, created_by) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(&code)
        .bind(file_id)
        .bind(user.id)
        .execute(&state.db)
        .await;
    match res {
        Ok(_) => Ok(Emote { id, code, url: format!("/files/{file_id}") }),
        Err(e) => {
            let e = Error::from(e);
            if e.is_unique_violation(None) {
                Err(Error::invalid("Diesen Code gibt es schon."))
            } else {
                Err(e)
            }
        }
    }
}

pub async fn delete_emote(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    if !user.is_admin() {
        return Err(Error::Forbidden);
    }
    sqlx::query("DELETE FROM watch_emotes WHERE id = $1").bind(id).execute(&state.db).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Shows (scheduled events)
// ---------------------------------------------------------------------------

/// Creating an event slightly in the past is allowed (it starts at once).
const START_GRACE_MS: i64 = 2 * 60_000;

pub async fn event_items(db: &PgPool, event_ids: &[Uuid]) -> Result<Vec<(Uuid, ShowItem)>> {
    let rows = sqlx::query(
        "SELECT i.id, i.event_id, i.media_id, i.intermission_after_ms, m.title, m.duration_ms, m.poster_path, m.status \
         FROM watch_event_items i JOIN watch_media m ON m.id = i.media_id \
         WHERE i.event_id = ANY($1) ORDER BY i.event_id, i.position",
    )
    .bind(event_ids)
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|r| {
            let media_id: Uuid = r.try_get("media_id")?;
            let poster: Option<String> = r.try_get("poster_path")?;
            Ok((
                r.try_get("event_id")?,
                ShowItem {
                    id: r.try_get("id")?,
                    media_id,
                    title: r.try_get("title")?,
                    duration_ms: r.try_get("duration_ms")?,
                    intermission_after_ms: r.try_get("intermission_after_ms")?,
                    poster_url: poster.map(|p| format!("/media/watch/{media_id}/{p}")),
                    ready: r.try_get::<String, _>("status")? == "ready",
                },
            ))
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Error::from)
}

fn event_from(r: &PgRow, items: Vec<ShowItem>) -> Result<EventInfo, sqlx::Error> {
    let phase: Option<String> = r.try_get("phase")?;
    Ok(EventInfo {
        id: r.try_get("id")?,
        room_id: r.try_get("room_id")?,
        title: r.try_get("title")?,
        starts_at_ms: r.try_get("starts_at")?,
        status: EventStatus::parse(&r.try_get::<String, _>("status")?),
        phase: phase.as_deref().and_then(ShowPhase::parse),
        item_index: r.try_get("item_index")?,
        items,
    })
}

const EVENT_COLS: &str = "id, room_id, title, starts_at, status, phase, item_index";

pub async fn events(state: &AppState, user: &AuthUser, room_id: Uuid, include_past: bool) -> Result<Vec<EventInfo>> {
    let (room, _) = accessible(&state.db, user, room_id).await?;
    let mut rows = sqlx::query(&format!(
        "SELECT {EVENT_COLS} FROM watch_events WHERE room_id = $1 AND status IN ('scheduled', 'live') ORDER BY starts_at"
    ))
    .bind(room.id)
    .fetch_all(&state.db)
    .await?;
    if include_past {
        rows.extend(
            sqlx::query(&format!(
                "SELECT {EVENT_COLS} FROM watch_events WHERE room_id = $1 AND status IN ('ended', 'cancelled') \
                 ORDER BY starts_at DESC LIMIT 50"
            ))
            .bind(room.id)
            .fetch_all(&state.db)
            .await?,
        );
    }
    let ids: Vec<Uuid> = rows.iter().filter_map(|r| r.try_get("id").ok()).collect();
    let mut items = event_items(&state.db, &ids).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let id: Uuid = r.try_get("id")?;
        let mine: Vec<ShowItem> = items.iter().filter(|(e, _)| *e == id).map(|(_, i)| i.clone()).collect();
        items.retain(|(e, _)| *e != id);
        out.push(event_from(r, mine)?);
    }
    Ok(out)
}

fn item_warnings(items: &[ShowItem]) -> Vec<String> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            if !item.ready {
                Some(format!(
                    "Film {} ({}) ist noch nicht verpackt – er wird übersprungen, falls er bis zur Vorstellung nicht bereit ist.",
                    i + 1,
                    item.title
                ))
            } else if item.duration_ms.is_none() {
                Some(format!(
                    "Film {} ({}) hat keine bekannte Länge – die Vorstellung schaltet danach nicht automatisch weiter.",
                    i + 1,
                    item.title
                ))
            } else {
                None
            }
        })
        .collect()
}

async fn validate_event(db: &PgPool, input: &EventInput) -> Result<String> {
    let title = input.title.trim().to_string();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(Error::invalid("Der Titel muss 1–120 Zeichen lang sein."));
    }
    if input.starts_at_ms < Utc::now().timestamp_millis() - START_GRACE_MS {
        return Err(Error::invalid("Der Beginn liegt in der Vergangenheit."));
    }
    if input.items.is_empty() || input.items.len() > 30 {
        return Err(Error::invalid("Füge 1–30 Filme hinzu."));
    }
    if input.items.iter().any(|i| !(0..=4 * 3_600_000).contains(&i.intermission_after_ms)) {
        return Err(Error::invalid("Eine Pause darf höchstens 4 Stunden dauern."));
    }
    let ids: Vec<Uuid> = input.items.iter().map(|i| i.media_id).collect();
    let known: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT id) FROM watch_media WHERE id = ANY($1) AND status <> 'archived'",
    )
    .bind(&ids)
    .fetch_one(db)
    .await?;
    let distinct = ids.iter().collect::<std::collections::HashSet<_>>().len() as i64;
    if known != distinct {
        return Err(Error::invalid("Unbekannter Film im Line-up."));
    }
    Ok(title)
}

async fn write_items(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, event_id: Uuid, input: &EventInput) -> Result<()> {
    sqlx::query("DELETE FROM watch_event_items WHERE event_id = $1")
        .bind(event_id)
        .execute(&mut **tx)
        .await?;
    for (pos, item) in input.items.iter().enumerate() {
        sqlx::query(
            "INSERT INTO watch_event_items (id, event_id, position, media_id, intermission_after_ms) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(event_id)
        .bind(pos as i32)
        .bind(item.media_id)
        .bind(item.intermission_after_ms)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn load_event(db: &PgPool, id: Uuid) -> Result<EventInfo> {
    let row = sqlx::query(&format!("SELECT {EVENT_COLS} FROM watch_events WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Vorstellung"))?;
    let items = event_items(db, &[id]).await?.into_iter().map(|(_, i)| i).collect();
    Ok(event_from(&row, items)?)
}

pub async fn create_event(state: &AppState, user: &AuthUser, room_id: Uuid, input: EventInput) -> Result<EventSaved> {
    let (room, _) = controlled(&state.db, user, room_id).await?;
    let title = validate_event(&state.db, &input).await?;
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO watch_events (id, room_id, title, starts_at, created_by) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(room.id)
        .bind(&title)
        .bind(input.starts_at_ms)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    write_items(&mut tx, id, &input).await?;
    tx.commit().await?;
    let event = load_event(&state.db, id).await?;
    let warnings = item_warnings(&event.items);
    watch_live::scheduler_kick(state);
    Ok(EventSaved { event, warnings })
}

async fn controlled_event(state: &AppState, user: &AuthUser, id: Uuid) -> Result<EventInfo> {
    let event = load_event(&state.db, id).await?;
    controlled(&state.db, user, event.room_id).await?;
    Ok(event)
}

pub async fn update_event(state: &AppState, user: &AuthUser, id: Uuid, input: EventInput) -> Result<EventSaved> {
    let event = controlled_event(state, user, id).await?;
    if event.status != EventStatus::Scheduled {
        return Err(Error::invalid("Die Vorstellung kann nicht mehr bearbeitet werden."));
    }
    let title = validate_event(&state.db, &input).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE watch_events SET title = $2, starts_at = $3 WHERE id = $1")
        .bind(id)
        .bind(&title)
        .bind(input.starts_at_ms)
        .execute(&mut *tx)
        .await?;
    write_items(&mut tx, id, &input).await?;
    tx.commit().await?;
    watch_live::refresh_show(state, event.room_id, id).await;
    let event = load_event(&state.db, id).await?;
    let warnings = item_warnings(&event.items);
    Ok(EventSaved { event, warnings })
}

pub async fn cancel_event(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let event = controlled_event(state, user, id).await?;
    if event.status != EventStatus::Scheduled {
        return Err(Error::invalid("Eine laufende Vorstellung wird in der Konsole beendet."));
    }
    sqlx::query("UPDATE watch_events SET status = 'cancelled' WHERE id = $1").bind(id).execute(&state.db).await?;
    watch_live::refresh_show(state, event.room_id, id).await;
    Ok(())
}

pub async fn delete_event(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let event = controlled_event(state, user, id).await?;
    if event.status == EventStatus::Live {
        return Err(Error::invalid("Die Vorstellung läuft – beende sie zuerst in der Konsole."));
    }
    sqlx::query("DELETE FROM watch_events WHERE id = $1").bind(id).execute(&state.db).await?;
    watch_live::refresh_show(state, event.room_id, id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Playlist persistence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PlaylistRow {
    pub id: Uuid,
    pub kind: SourceKind,
    pub media_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub url: Option<String>,
    pub title: String,
    pub duration_ms: Option<i64>,
    pub thumb_url: Option<String>,
    pub added_by: Option<shared::UserSummary>,
}

impl PlaylistRow {
    pub fn view(&self) -> PlaylistItem {
        PlaylistItem {
            id: self.id,
            title: self.title.clone(),
            kind: self.kind,
            duration_ms: self.duration_ms,
            thumb_url: self.thumb_url.clone(),
            added_by: self.added_by.clone(),
        }
    }
}

pub async fn playlist(db: &PgPool, room_id: Uuid) -> Result<Vec<PlaylistRow>> {
    let rows = sqlx::query(&format!(
        "SELECT p.id, p.kind, p.media_id, p.file_id, p.url, p.title, p.duration_ms, p.thumb_url, \
           p.added_by AS added_ref, {} \
         FROM watch_playlist p LEFT JOIN users u ON u.id = p.added_by WHERE p.room_id = $1 \
         ORDER BY p.position, p.created_at LIMIT 500",
        user_cols("u", "u")
    ))
    .bind(room_id)
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(PlaylistRow {
                id: r.try_get("id")?,
                kind: SourceKind::parse(&r.try_get::<String, _>("kind")?),
                media_id: r.try_get("media_id")?,
                file_id: r.try_get("file_id")?,
                url: r.try_get("url")?,
                title: r.try_get("title")?,
                duration_ms: r.try_get("duration_ms")?,
                thumb_url: r.try_get("thumb_url")?,
                added_by: crate::users::opt_user(r, "u")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Error::from)
}

pub struct NewItem {
    pub kind: SourceKind,
    pub media_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub url: Option<String>,
    pub title: String,
    pub duration_ms: Option<i64>,
    pub thumb_url: Option<String>,
}

pub async fn insert_playlist_item(db: &PgPool, room_id: Uuid, user: &AuthUser, item: NewItem, next: bool) -> Result<Uuid> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM watch_playlist WHERE room_id = $1")
        .bind(room_id)
        .fetch_one(db)
        .await?;
    if count >= 500 {
        return Err(Error::invalid("Die Playlist ist voll (500 Einträge)."));
    }
    let position: f64 = if next {
        sqlx::query_scalar("SELECT coalesce(min(position), 1) - 1 FROM watch_playlist WHERE room_id = $1")
    } else {
        sqlx::query_scalar("SELECT coalesce(max(position), 0) + 1 FROM watch_playlist WHERE room_id = $1")
    }
    .bind(room_id)
    .fetch_one(db)
    .await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO watch_playlist (id, room_id, position, kind, media_id, file_id, url, title, duration_ms, thumb_url, added_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(room_id)
    .bind(position)
    .bind(item.kind.as_str())
    .bind(item.media_id)
    .bind(item.file_id)
    .bind(&item.url)
    .bind(item.title.chars().take(200).collect::<String>())
    .bind(item.duration_ms)
    .bind(&item.thumb_url)
    .bind(user.id)
    .execute(db)
    .await?;
    Ok(id)
}

pub async fn remove_playlist_item(db: &PgPool, room_id: Uuid, item_id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM watch_playlist WHERE id = $1 AND room_id = $2")
        .bind(item_id)
        .bind(room_id)
        .execute(db)
        .await?
        .rows_affected()
        > 0)
}

/// Moves an item so it lands before `before` (or to the end).
pub async fn move_playlist_item(db: &PgPool, room_id: Uuid, item_id: Uuid, before: Option<Uuid>) -> Result<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM watch_playlist WHERE room_id = $1 ORDER BY position, created_at")
        .bind(room_id)
        .fetch_all(db)
        .await?;
    if !ids.contains(&item_id) {
        return Err(Error::NotFound("Eintrag"));
    }
    let mut order: Vec<Uuid> = ids.into_iter().filter(|i| *i != item_id).collect();
    let at = before.and_then(|b| order.iter().position(|i| *i == b)).unwrap_or(order.len());
    order.insert(at, item_id);
    let positions: Vec<f64> = (0..order.len()).map(|i| i as f64).collect();
    sqlx::query(
        "UPDATE watch_playlist p SET position = o.pos FROM unnest($2::uuid[], $3::float8[]) AS o(id, pos) \
         WHERE p.id = o.id AND p.room_id = $1",
    )
    .bind(room_id)
    .bind(&order)
    .bind(&positions)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn shuffle_playlist(db: &PgPool, room_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE watch_playlist SET position = random() * 1000 WHERE room_id = $1")
        .bind(room_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn clear_playlist(db: &PgPool, room_id: Uuid, keep: Option<Uuid>) -> Result<()> {
    sqlx::query("DELETE FROM watch_playlist WHERE room_id = $1 AND id IS DISTINCT FROM $2")
        .bind(room_id)
        .bind(keep)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_item_duration(db: &PgPool, item_id: Uuid, duration_ms: i64) -> Result<()> {
    sqlx::query("UPDATE watch_playlist SET duration_ms = $2 WHERE id = $1 AND duration_ms IS NULL")
        .bind(item_id)
        .bind(duration_ms)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn touch_room(db: &PgPool, room_id: Uuid) {
    let _ = sqlx::query("UPDATE watch_rooms SET last_active_at = now() WHERE id = $1")
        .bind(room_id)
        .execute(db)
        .await;
}

pub async fn total_rooms(db: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM watch_rooms").fetch_one(db).await?)
}

/// The user's own video files that others can see (playlist source).
pub async fn my_videos(db: &PgPool, user: &AuthUser) -> Result<Vec<shared::files::FileView>> {
    let rows = sqlx::query(&format!(
        "SELECT {}{} WHERE f.owner_id = $1 AND f.deleted_at IS NULL AND f.mime_type LIKE 'video/%'          ORDER BY f.created_at DESC LIMIT 200",
        crate::files::file_cols(),
        crate::files::FILE_JOIN
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(crate::files::record_from)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|r| r.view)
        .collect())
}
