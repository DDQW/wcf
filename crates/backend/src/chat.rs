//! Chat: channels, direct messages, group DMs and realtime messages.

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use shared::chat::{
    normalize_channel_name, ChannelDetail, ChannelKind, ChannelView, ChatMemberView,
    ChatMessageView, ChatRole, MessageKind, MessagePage, NewChannel, ReplyPreview,
    MAX_ATTACHMENTS, MAX_MESSAGE_LEN, MESSAGES_PER_PAGE,
};
use shared::files::{FilePurpose, FileView, Visibility};
use shared::ws::{ServerMsg, Topic};
use shared::{AuthUser, Cursor, NotificationKind, SubjectType, UserSummary};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::users::{opt_user, user_cols, user_from};
use crate::{files, markdown, modlog, reactions};

/// Membership + channel info used for permission checks.
#[derive(Debug, Clone)]
pub struct Access {
    pub kind: ChannelKind,
    pub is_archived: bool,
    pub role: Option<ChatRole>,
    pub name: String,
}

impl Access {
    pub fn is_member(&self) -> bool {
        self.role.is_some()
    }

    pub fn can_read(&self, user: &AuthUser) -> bool {
        match self.kind {
            ChannelKind::Public => true,
            ChannelKind::Room => true,
            _ => self.is_member() || (user.is_mod() && self.kind == ChannelKind::Private),
        }
    }

    pub fn can_post(&self, _user: &AuthUser) -> bool {
        if self.is_archived {
            return false;
        }
        match self.kind {
            ChannelKind::Public | ChannelKind::Room => true,
            _ => self.is_member(),
        }
    }

    pub fn can_manage(&self, user: &AuthUser) -> bool {
        self.role.is_some_and(|r| r.can_manage())
            || (user.is_mod() && matches!(self.kind, ChannelKind::Public | ChannelKind::Private))
    }

    pub fn can_moderate(&self, user: &AuthUser) -> bool {
        self.can_manage(user) || (user.is_mod() && self.kind != ChannelKind::Direct)
    }
}

pub async fn access(db: &PgPool, channel_id: Uuid, user_id: Uuid) -> Result<Option<Access>> {
    let row = sqlx::query(
        "SELECT c.kind, c.is_archived, c.name, m.role FROM chat_channels c \
         LEFT JOIN chat_members m ON m.channel_id = c.id AND m.user_id = $2 WHERE c.id = $1",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let kind: String = r.try_get("kind")?;
    let role: Option<String> = r.try_get("role")?;
    Ok(Some(Access {
        kind: ChannelKind::parse(&kind),
        is_archived: r.try_get("is_archived")?,
        role: role.map(|r| ChatRole::parse(&r)),
        name: r.try_get("name")?,
    }))
}

async fn readable(db: &PgPool, user: &AuthUser, channel_id: Uuid) -> Result<Access> {
    let acc = access(db, channel_id, user.id).await?.ok_or(Error::NotFound("Kanal"))?;
    if !acc.can_read(user) {
        return Err(Error::NotFound("Kanal"));
    }
    Ok(acc)
}

pub async fn can_subscribe(db: &PgPool, user: Option<&AuthUser>, channel_id: Uuid) -> bool {
    let Some(user) = user else { return false };
    matches!(access(db, channel_id, user.id).await, Ok(Some(a)) if a.can_read(user))
}

// ---------------------------------------------------------------------------
// Channel listing
// ---------------------------------------------------------------------------

const CHANNEL_SELECT: &str = "
    SELECT c.id, c.kind, c.name, c.topic, c.is_archived, c.last_message_at,
           (SELECT count(*) FROM chat_members x WHERE x.channel_id = c.id) AS member_count,
           m.role, m.muted, m.last_read_at,
           CASE WHEN m.user_id IS NULL THEN 0 ELSE (
               SELECT count(*) FROM chat_messages cm WHERE cm.channel_id = c.id
                 AND cm.created_at > m.last_read_at AND cm.deleted_at IS NULL
                 AND cm.author_id IS DISTINCT FROM $1) END AS unread,
           CASE WHEN m.user_id IS NULL THEN 0 ELSE (
               SELECT count(*) FROM chat_messages cm WHERE cm.channel_id = c.id
                 AND cm.created_at > m.last_read_at AND cm.deleted_at IS NULL
                 AND $1 = ANY(cm.mentions)) END AS mentions
    FROM chat_channels c
    LEFT JOIN chat_members m ON m.channel_id = c.id AND m.user_id = $1
";

async fn channel_from(db: &PgPool, r: &PgRow, viewer: Uuid) -> Result<ChannelView> {
    let id: Uuid = r.try_get("id")?;
    let kind = ChannelKind::parse(&r.try_get::<String, _>("kind")?);
    let role: Option<String> = r.try_get("role")?;
    let mut name: String = r.try_get("name")?;
    let mut dm_user = None;
    if kind.is_dm() {
        let others: Vec<UserSummary> = dm_others(db, id, viewer).await?;
        if kind == ChannelKind::Direct {
            dm_user = others.first().cloned();
        }
        if name.trim().is_empty() || kind == ChannelKind::Direct {
            name = if others.is_empty() {
                "Nur du".to_string()
            } else {
                others.iter().map(|u| u.display_name.clone()).collect::<Vec<_>>().join(", ")
            };
        }
    }
    Ok(ChannelView {
        id,
        kind,
        name,
        topic: r.try_get("topic")?,
        is_archived: r.try_get("is_archived")?,
        member_count: r.try_get("member_count")?,
        unread: r.try_get("unread")?,
        mentions: r.try_get("mentions")?,
        last_message_at: r.try_get("last_message_at")?,
        muted: r.try_get::<Option<bool>, _>("muted")?.unwrap_or(false),
        joined: role.is_some(),
        my_role: role.map(|r| ChatRole::parse(&r)),
        dm_user,
    })
}

async fn dm_others(db: &PgPool, channel_id: Uuid, viewer: Uuid) -> Result<Vec<UserSummary>> {
    let rows = sqlx::query(&format!(
        "SELECT {} FROM chat_members m JOIN users u ON u.id = m.user_id \
         WHERE m.channel_id = $1 AND m.user_id <> $2 ORDER BY u.display_name LIMIT 8",
        user_cols("u", "u")
    ))
    .bind(channel_id)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(|r| user_from(r, "u")).collect::<Result<_, _>>()?)
}

/// Channels the user belongs to (sidebar), newest activity first.
pub async fn my_channels(db: &PgPool, user: &AuthUser) -> Result<Vec<ChannelView>> {
    let rows = sqlx::query(&format!(
        "{CHANNEL_SELECT} WHERE m.user_id = $1 AND NOT m.hidden AND c.kind <> 'room' \
         ORDER BY c.kind IN ('direct', 'group'), c.name, c.last_message_at DESC NULLS LAST"
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(channel_from(db, r, user.id).await?);
    }
    Ok(out)
}

/// Public channels for the channel browser.
pub async fn browse(db: &PgPool, user: &AuthUser) -> Result<Vec<ChannelView>> {
    let rows = sqlx::query(&format!(
        "{CHANNEL_SELECT} WHERE c.kind = 'public' AND NOT c.is_archived ORDER BY c.name"
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(channel_from(db, r, user.id).await?);
    }
    Ok(out)
}

pub async fn channel_view(db: &PgPool, user: &AuthUser, channel_id: Uuid) -> Result<ChannelView> {
    let row = sqlx::query(&format!("{CHANNEL_SELECT} WHERE c.id = $2"))
        .bind(user.id)
        .bind(channel_id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Kanal"))?;
    channel_from(db, &row, user.id).await
}

pub async fn detail(db: &PgPool, user: &AuthUser, channel_id: Uuid) -> Result<ChannelDetail> {
    let acc = readable(db, user, channel_id).await?;
    let channel = channel_view(db, user, channel_id).await?;
    let last_read_at: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT last_read_at FROM chat_members WHERE channel_id = $1 AND user_id = $2",
    )
    .bind(channel_id)
    .bind(user.id)
    .fetch_optional(db)
    .await?;
    let rows = sqlx::query(&format!(
        "SELECT {}, m.role, m.joined_at FROM chat_members m JOIN users u ON u.id = m.user_id \
         WHERE m.channel_id = $1 ORDER BY m.role = 'member', lower(u.display_name) LIMIT 500",
        user_cols("u", "u")
    ))
    .bind(channel_id)
    .fetch_all(db)
    .await?;
    let members = rows
        .iter()
        .map(|r| -> Result<ChatMemberView> {
            Ok(ChatMemberView {
                user: user_from(r, "u")?,
                role: ChatRole::parse(&r.try_get::<String, _>("role")?),
                joined_at: r.try_get("joined_at")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ChannelDetail {
        can_post: acc.can_post(user),
        can_manage: acc.can_manage(user),
        can_moderate: acc.can_moderate(user),
        channel,
        members,
        last_read_at,
    })
}

/// Total unread messages over all joined channels (nav badge).
pub async fn unread_total(db: &PgPool, user: &AuthUser) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM chat_messages cm JOIN chat_members m \
            ON m.channel_id = cm.channel_id AND m.user_id = $1 \
         JOIN chat_channels c ON c.id = cm.channel_id \
         WHERE cm.created_at > m.last_read_at AND cm.deleted_at IS NULL \
           AND cm.author_id IS DISTINCT FROM $1 AND NOT m.muted AND c.kind <> 'room'",
    )
    .bind(user.id)
    .fetch_one(db)
    .await?)
}

// ---------------------------------------------------------------------------
// Channel management
// ---------------------------------------------------------------------------

fn validate_topic(topic: &str) -> Result<String> {
    let t = topic.trim();
    if t.chars().count() > 250 {
        return Err(Error::invalid("Das Thema ist zu lang (max. 250 Zeichen)."));
    }
    Ok(t.to_string())
}

pub async fn create_channel(state: &AppState, user: &AuthUser, input: NewChannel) -> Result<Uuid> {
    if !user.is_mod() && !state.settings().members_create_channels {
        return Err(Error::invalid("Nur Moderatoren dürfen Kanäle erstellen."));
    }
    state
        .limiter
        .hit(&format!("channel:{}", user.id), 10, StdDuration::from_secs(3600))?;
    let name = normalize_channel_name(&input.name);
    if name.chars().count() < 2 {
        return Err(Error::invalid("Der Kanalname muss mindestens 2 Zeichen haben (a–z, 0–9, -)."));
    }
    let topic = validate_topic(&input.topic)?;
    if !input.private {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE kind = 'public' AND name = $1)",
        )
        .bind(&name)
        .fetch_one(&state.db)
        .await?;
        if exists {
            return Err(Error::invalid("Einen öffentlichen Kanal mit diesem Namen gibt es schon."));
        }
    }
    let id = Uuid::now_v7();
    let kind = if input.private { "private" } else { "public" };
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO chat_channels (id, kind, name, topic, created_by, last_message_at) VALUES ($1, $2, $3, $4, $5, now())",
    )
    .bind(id)
    .bind(kind)
    .bind(&name)
    .bind(&topic)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO chat_members (channel_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    for m in input.members.iter().filter(|m| **m != user.id).take(200) {
        sqlx::query("INSERT INTO chat_members (channel_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(m)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    for m in input.members.iter().filter(|m| **m != user.id) {
        state.hub.send_user(*m, &ServerMsg::ChatChannelUpdated { channel_id: id });
    }
    Ok(id)
}

pub async fn update_channel(state: &AppState, user: &AuthUser, channel_id: Uuid, name: &str, topic: &str) -> Result<()> {
    let acc = readable(&state.db, user, channel_id).await?;
    if !acc.can_manage(user) {
        return Err(Error::Forbidden);
    }
    let topic = validate_topic(topic)?;
    let new_name = if acc.kind.is_dm() {
        name.trim().chars().take(60).collect::<String>()
    } else {
        let n = normalize_channel_name(name);
        if n.chars().count() < 2 {
            return Err(Error::invalid("Der Kanalname muss mindestens 2 Zeichen haben."));
        }
        n
    };
    sqlx::query("UPDATE chat_channels SET name = $2, topic = $3 WHERE id = $1")
        .bind(channel_id)
        .bind(&new_name)
        .bind(&topic)
        .execute(&state.db)
        .await?;
    system_message(state, channel_id, &format!("{} hat den Kanal aktualisiert.", user.display_name)).await?;
    state
        .hub
        .publish(&Topic::Chat(channel_id), &ServerMsg::ChatChannelUpdated { channel_id });
    Ok(())
}

pub async fn set_archived(state: &AppState, user: &AuthUser, channel_id: Uuid, archived: bool) -> Result<()> {
    let acc = readable(&state.db, user, channel_id).await?;
    if !acc.can_manage(user) || acc.kind.is_dm() {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE chat_channels SET is_archived = $2 WHERE id = $1")
        .bind(channel_id)
        .bind(archived)
        .execute(&state.db)
        .await?;
    state
        .hub
        .publish(&Topic::Chat(channel_id), &ServerMsg::ChatChannelUpdated { channel_id });
    Ok(())
}

pub async fn join(state: &AppState, user: &AuthUser, channel_id: Uuid) -> Result<()> {
    let acc = access(&state.db, channel_id, user.id).await?.ok_or(Error::NotFound("Kanal"))?;
    if acc.kind != ChannelKind::Public {
        return Err(Error::Forbidden);
    }
    let inserted = sqlx::query("INSERT INTO chat_members (channel_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(channel_id)
        .bind(user.id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if inserted > 0 {
        system_message(state, channel_id, &format!("{} ist dem Kanal beigetreten.", user.display_name)).await?;
    }
    Ok(())
}

pub async fn leave(state: &AppState, user: &AuthUser, channel_id: Uuid) -> Result<()> {
    let acc = access(&state.db, channel_id, user.id).await?.ok_or(Error::NotFound("Kanal"))?;
    if acc.kind == ChannelKind::Direct {
        // A direct message cannot be left – it is hidden instead.
        sqlx::query("UPDATE chat_members SET hidden = TRUE WHERE channel_id = $1 AND user_id = $2")
            .bind(channel_id)
            .bind(user.id)
            .execute(&state.db)
            .await?;
        return Ok(());
    }
    if acc.role == Some(ChatRole::Owner) {
        // Hand ownership to the longest-standing member, if any.
        sqlx::query(
            "UPDATE chat_members SET role = 'owner' WHERE channel_id = $1 AND user_id = ( \
                SELECT user_id FROM chat_members WHERE channel_id = $1 AND user_id <> $2 \
                ORDER BY role = 'admin' DESC, joined_at LIMIT 1)",
        )
        .bind(channel_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    }
    sqlx::query("DELETE FROM chat_members WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    system_message(state, channel_id, &format!("{} hat den Kanal verlassen.", user.display_name)).await?;
    Ok(())
}

pub async fn add_members(state: &AppState, user: &AuthUser, channel_id: Uuid, ids: &[Uuid]) -> Result<()> {
    let acc = readable(&state.db, user, channel_id).await?;
    let allowed = match acc.kind {
        ChannelKind::Group => acc.is_member(),
        ChannelKind::Private | ChannelKind::Public => acc.can_manage(user),
        _ => false,
    };
    if !allowed {
        return Err(Error::Forbidden);
    }
    let names = crate::users::summaries(&state.db, ids).await?;
    for (id, u) in &names {
        let added = sqlx::query(
            "INSERT INTO chat_members (channel_id, user_id) VALUES ($1, $2) \
             ON CONFLICT (channel_id, user_id) DO UPDATE SET hidden = FALSE",
        )
        .bind(channel_id)
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
        if added > 0 {
            system_message(state, channel_id, &format!("{} hat {} hinzugefügt.", user.display_name, u.display_name)).await?;
            state.hub.send_user(*id, &ServerMsg::ChatChannelUpdated { channel_id });
        }
    }
    Ok(())
}

pub async fn remove_member(state: &AppState, user: &AuthUser, channel_id: Uuid, target: Uuid) -> Result<()> {
    let acc = readable(&state.db, user, channel_id).await?;
    if !acc.can_manage(user) || acc.kind == ChannelKind::Direct {
        return Err(Error::Forbidden);
    }
    let target_role: Option<String> =
        sqlx::query_scalar("SELECT role FROM chat_members WHERE channel_id = $1 AND user_id = $2")
            .bind(channel_id)
            .bind(target)
            .fetch_optional(&state.db)
            .await?;
    if target_role.as_deref() == Some("owner") {
        return Err(Error::invalid("Der Inhaber kann nicht entfernt werden."));
    }
    sqlx::query("DELETE FROM chat_members WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(target)
        .execute(&state.db)
        .await?;
    if let Some(u) = crate::users::summary(&state.db, target).await? {
        system_message(state, channel_id, &format!("{} hat {} entfernt.", user.display_name, u.display_name)).await?;
    }
    state.hub.send_user(target, &ServerMsg::ChatChannelUpdated { channel_id });
    Ok(())
}

pub async fn set_member_role(state: &AppState, user: &AuthUser, channel_id: Uuid, target: Uuid, role: ChatRole) -> Result<()> {
    let acc = readable(&state.db, user, channel_id).await?;
    if !(acc.role == Some(ChatRole::Owner) || (user.is_mod() && !acc.kind.is_dm())) {
        return Err(Error::Forbidden);
    }
    if role == ChatRole::Owner {
        return Err(Error::invalid("Inhaberschaft kann nicht vergeben werden."));
    }
    sqlx::query("UPDATE chat_members SET role = $3 WHERE channel_id = $1 AND user_id = $2 AND role <> 'owner'")
        .bind(channel_id)
        .bind(target)
        .bind(role.as_str())
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn set_muted(db: &PgPool, user: &AuthUser, channel_id: Uuid, muted: bool) -> Result<()> {
    sqlx::query("UPDATE chat_members SET muted = $3 WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(user.id)
        .bind(muted)
        .execute(db)
        .await?;
    Ok(())
}

/// Finds or creates the direct-message channel between two users.
pub async fn open_dm(state: &AppState, user: &AuthUser, other: Uuid) -> Result<Uuid> {
    if other == user.id {
        return Err(Error::invalid("Du kannst dir selbst keine Nachricht schreiben."));
    }
    crate::users::summary(&state.db, other).await?.ok_or(Error::NotFound("Benutzer"))?;
    let (a, b) = if user.id < other { (user.id, other) } else { (other, user.id) };
    let key = format!("{a}:{b}");
    if let Some(id) = sqlx::query_scalar::<_, Uuid>("SELECT id FROM chat_channels WHERE dm_key = $1")
        .bind(&key)
        .fetch_optional(&state.db)
        .await?
    {
        sqlx::query("UPDATE chat_members SET hidden = FALSE WHERE channel_id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.id)
            .execute(&state.db)
            .await?;
        return Ok(id);
    }
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO chat_channels (id, kind, dm_key, created_by, last_message_at) VALUES ($1, 'direct', $2, $3, now())")
        .bind(id)
        .bind(&key)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    for uid in [a, b] {
        sqlx::query("INSERT INTO chat_members (channel_id, user_id, role) VALUES ($1, $2, 'member')")
            .bind(id)
            .bind(uid)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(id)
}

pub async fn create_group(state: &AppState, user: &AuthUser, members: &[Uuid], name: &str) -> Result<Uuid> {
    let mut ids: Vec<Uuid> = members.iter().copied().filter(|m| *m != user.id).collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(Error::invalid("Bitte mindestens eine Person auswählen."));
    }
    if ids.len() == 1 {
        return open_dm(state, user, ids[0]).await;
    }
    if ids.len() > 50 {
        return Err(Error::invalid("Gruppen sind auf 50 Personen begrenzt."));
    }
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO chat_channels (id, kind, name, created_by, last_message_at) VALUES ($1, 'group', $2, $3, now())")
        .bind(id)
        .bind(name.trim().chars().take(60).collect::<String>())
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO chat_members (channel_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    for m in &ids {
        sqlx::query("INSERT INTO chat_members (channel_id, user_id) VALUES ($1, $2)")
            .bind(id)
            .bind(m)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    for m in &ids {
        state.hub.send_user(*m, &ServerMsg::ChatChannelUpdated { channel_id: id });
    }
    Ok(id)
}

/// Chat channel for a room (watch/call); created on first use.
pub async fn room_channel(db: &PgPool, room_id: Uuid, name: &str) -> Result<Uuid> {
    let key = format!("room:{room_id}");
    if let Some(id) = sqlx::query_scalar::<_, Uuid>("SELECT id FROM chat_channels WHERE dm_key = $1")
        .bind(&key)
        .fetch_optional(db)
        .await?
    {
        return Ok(id);
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat_channels (id, kind, name, dm_key, last_message_at) VALUES ($1, 'room', $2, $3, now()) \
         ON CONFLICT (dm_key) DO NOTHING",
    )
    .bind(id)
    .bind(name)
    .bind(&key)
    .execute(db)
    .await?;
    Ok(sqlx::query_scalar("SELECT id FROM chat_channels WHERE dm_key = $1")
        .bind(&key)
        .fetch_one(db)
        .await?)
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

const MESSAGE_SELECT: &str = "
    SELECT m.id, m.channel_id, m.kind, m.body_html, m.created_at, m.edited_at, m.deleted_at,
           m.pinned_at, m.attachments, m.mentions, m.reply_to_id,
           a.id AS author_id, a.username::text AS author_username, a.display_name AS author_display_name,
           a.avatar_file_id AS author_avatar, a.role AS author_role,
           r.body_md AS reply_body, r.deleted_at AS reply_deleted, ra.display_name AS reply_author
    FROM chat_messages m
    LEFT JOIN users a ON a.id = m.author_id
    LEFT JOIN chat_messages r ON r.id = m.reply_to_id
    LEFT JOIN users ra ON ra.id = r.author_id
";

struct RawMessage {
    view: ChatMessageView,
    attachment_ids: Vec<Uuid>,
}

fn message_from(r: &PgRow, viewer: Uuid) -> Result<RawMessage> {
    let deleted_at: Option<DateTime<Utc>> = r.try_get("deleted_at")?;
    let deleted = deleted_at.is_some();
    let reply_id: Option<Uuid> = r.try_get("reply_to_id")?;
    let reply_to = match reply_id {
        Some(id) => {
            let body: Option<String> = r.try_get("reply_body")?;
            let reply_deleted: Option<DateTime<Utc>> = r.try_get("reply_deleted")?;
            Some(ReplyPreview {
                id,
                author_name: r.try_get::<Option<String>, _>("reply_author")?.unwrap_or_default(),
                excerpt: if reply_deleted.is_some() {
                    "(gelöscht)".into()
                } else {
                    markdown::excerpt(&body.unwrap_or_default(), 120)
                },
            })
        }
        None => None,
    };
    let mentions: Vec<Uuid> = r.try_get("mentions")?;
    let kind: String = r.try_get("kind")?;
    let pinned_at: Option<DateTime<Utc>> = r.try_get("pinned_at")?;
    Ok(RawMessage {
        attachment_ids: if deleted { Vec::new() } else { r.try_get("attachments")? },
        view: ChatMessageView {
            id: r.try_get("id")?,
            channel_id: r.try_get("channel_id")?,
            kind: if kind == "system" { MessageKind::System } else { MessageKind::Text },
            author: opt_user(r, "author")?,
            body_html: if deleted { String::new() } else { r.try_get("body_html")? },
            created_at: r.try_get("created_at")?,
            edited_at: r.try_get("edited_at")?,
            deleted,
            pinned: pinned_at.is_some(),
            reply_to,
            attachments: Vec::new(),
            reactions: Vec::new(),
            mentions_me: mentions.contains(&viewer),
        },
    })
}

async fn finish_messages(db: &PgPool, raw: Vec<RawMessage>, viewer: Uuid) -> Result<Vec<ChatMessageView>> {
    let all_files: Vec<Uuid> = raw.iter().flat_map(|m| m.attachment_ids.iter().copied()).collect();
    let file_map: HashMap<Uuid, FileView> = files::get_records(db, &all_files)
        .await?
        .into_iter()
        .filter(|r| r.view.deleted_at.is_none())
        .map(|r| (r.view.id, r.view))
        .collect();
    let ids: Vec<Uuid> = raw.iter().map(|m| m.view.id).collect();
    let mut counts = reactions::counts_for(db, SubjectType::ChatMessage, &ids, Some(viewer)).await?;
    Ok(raw
        .into_iter()
        .map(|mut m| {
            m.view.attachments = m.attachment_ids.iter().filter_map(|id| file_map.get(id).cloned()).collect();
            m.view.reactions = counts.remove(&m.view.id).unwrap_or_default();
            m.view
        })
        .collect())
}

pub async fn message_view(db: &PgPool, viewer: Uuid, id: Uuid) -> Result<ChatMessageView> {
    let row = sqlx::query(&format!("{MESSAGE_SELECT} WHERE m.id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Nachricht"))?;
    let raw = message_from(&row, viewer)?;
    Ok(finish_messages(db, vec![raw], viewer).await?.remove(0))
}

/// Loads a page of messages: latest, older than / newer than a cursor, or
/// centered around a message (jump to search result / pin).
pub async fn list_messages(
    db: &PgPool,
    user: &AuthUser,
    channel_id: Uuid,
    before: Option<String>,
    after: Option<String>,
    around: Option<Uuid>,
) -> Result<MessagePage> {
    readable(db, user, channel_id).await?;
    let n = MESSAGES_PER_PAGE;
    let mut older_cursor = None;
    let mut newer_cursor = None;
    let rows: Vec<PgRow>;

    if let Some(target) = around {
        let pivot: Option<(DateTime<Utc>, Uuid)> =
            sqlx::query_as("SELECT created_at, id FROM chat_messages WHERE id = $1 AND channel_id = $2")
                .bind(target)
                .bind(channel_id)
                .fetch_optional(db)
                .await?;
        let (ts, id) = pivot.ok_or(Error::NotFound("Nachricht"))?;
        let mut older = sqlx::query(&format!(
            "{MESSAGE_SELECT} WHERE m.channel_id = $1 AND (m.created_at, m.id) < ($2, $3) \
             ORDER BY m.created_at DESC, m.id DESC LIMIT $4"
        ))
        .bind(channel_id)
        .bind(ts)
        .bind(id)
        .bind(n / 2 + 1)
        .fetch_all(db)
        .await?;
        if older.len() as i64 > n / 2 {
            older.truncate((n / 2) as usize);
            older_cursor = Some(String::new());
        }
        older.reverse();
        let mut newer = sqlx::query(&format!(
            "{MESSAGE_SELECT} WHERE m.channel_id = $1 AND (m.created_at, m.id) >= ($2, $3) \
             ORDER BY m.created_at, m.id LIMIT $4"
        ))
        .bind(channel_id)
        .bind(ts)
        .bind(id)
        .bind(n / 2 + 1)
        .fetch_all(db)
        .await?;
        if newer.len() as i64 > n / 2 {
            newer.truncate((n / 2) as usize);
            newer_cursor = Some(String::new());
        }
        older.extend(newer);
        rows = older;
    } else if let Some(c) = after.as_deref().and_then(|c| Cursor::decode(c).ok()) {
        let mut list = sqlx::query(&format!(
            "{MESSAGE_SELECT} WHERE m.channel_id = $1 AND (m.created_at, m.id) > ($2, $3) \
             ORDER BY m.created_at, m.id LIMIT $4"
        ))
        .bind(channel_id)
        .bind(c.ts)
        .bind(c.id)
        .bind(n + 1)
        .fetch_all(db)
        .await?;
        if list.len() as i64 > n {
            list.truncate(n as usize);
            newer_cursor = Some(String::new());
        }
        rows = list;
    } else {
        let cursor = before.as_deref().and_then(|c| Cursor::decode(c).ok());
        let mut list = sqlx::query(&format!(
            "{MESSAGE_SELECT} WHERE m.channel_id = $1 \
               AND ($2::timestamptz IS NULL OR (m.created_at, m.id) < ($2, $3)) \
             ORDER BY m.created_at DESC, m.id DESC LIMIT $4"
        ))
        .bind(channel_id)
        .bind(cursor.map(|c| c.ts))
        .bind(cursor.map(|c| c.id))
        .bind(n + 1)
        .fetch_all(db)
        .await?;
        if list.len() as i64 > n {
            list.truncate(n as usize);
            older_cursor = Some(String::new());
        }
        list.reverse();
        rows = list;
    }

    let raw = rows.iter().map(|r| message_from(r, user.id)).collect::<Result<Vec<_>>>()?;
    let messages = finish_messages(db, raw, user.id).await?;
    if older_cursor.is_some() {
        older_cursor = messages.first().map(|m| Cursor::new(m.created_at, m.id).encode());
    }
    if newer_cursor.is_some() {
        newer_cursor = messages.last().map(|m| Cursor::new(m.created_at, m.id).encode());
    }
    Ok(MessagePage {
        messages,
        older_cursor,
        newer_cursor,
    })
}

async fn system_message(state: &AppState, channel_id: Uuid, text: &str) -> Result<()> {
    let id = Uuid::now_v7();
    let html = markdown::escape_html(text);
    sqlx::query(
        "INSERT INTO chat_messages (id, channel_id, kind, body_md, body_html) VALUES ($1, $2, 'system', $3, $4)",
    )
    .bind(id)
    .bind(channel_id)
    .bind(text)
    .bind(format!("<p>{html}</p>"))
    .execute(&state.db)
    .await?;
    let msg = ChatMessageView {
        id,
        channel_id,
        kind: MessageKind::System,
        author: None,
        body_html: format!("<p>{html}</p>"),
        created_at: Utc::now(),
        edited_at: None,
        deleted: false,
        pinned: false,
        reply_to: None,
        attachments: Vec::new(),
        reactions: Vec::new(),
        mentions_me: false,
    };
    state
        .hub
        .publish(&Topic::Chat(channel_id), &ServerMsg::ChatMessage { message: msg });
    Ok(())
}

pub async fn send(
    state: &AppState,
    user: &AuthUser,
    channel_id: Uuid,
    body_md: &str,
    reply_to: Option<Uuid>,
    attachments: Vec<Uuid>,
) -> Result<ChatMessageView> {
    state
        .limiter
        .hit(&format!("chat:{}", user.id), 20, StdDuration::from_secs(10))?;
    let acc = readable(&state.db, user, channel_id).await?;
    if !acc.can_post(user) {
        return Err(Error::invalid(if acc.is_archived {
            "Dieser Kanal ist archiviert."
        } else {
            "Du kannst in diesem Kanal nicht schreiben."
        }));
    }
    let body = body_md.trim_end();
    if body.trim().is_empty() && attachments.is_empty() {
        return Err(Error::invalid("Die Nachricht ist leer."));
    }
    if body.chars().count() > MAX_MESSAGE_LEN {
        return Err(Error::invalid("Die Nachricht ist zu lang."));
    }
    if attachments.len() > MAX_ATTACHMENTS {
        return Err(Error::invalid("Zu viele Anhänge (max. 10)."));
    }
    if !attachments.is_empty() {
        let recs = files::get_records(&state.db, &attachments).await?;
        if recs.len() != attachments.len() || recs.iter().any(|r| r.owner_id != user.id) {
            return Err(Error::invalid("Ungültiger Anhang."));
        }
    }
    if let Some(r) = reply_to {
        let same: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat_messages WHERE id = $1 AND channel_id = $2)")
            .bind(r)
            .bind(channel_id)
            .fetch_one(&state.db)
            .await?;
        if !same {
            return Err(Error::invalid("Die Nachricht, auf die du antwortest, gibt es nicht mehr."));
        }
    }
    // Posting in a public channel joins it.
    if !acc.is_member() && acc.kind == ChannelKind::Public {
        sqlx::query("INSERT INTO chat_members (channel_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(channel_id)
            .bind(user.id)
            .execute(&state.db)
            .await?;
    }
    let rendered = markdown::render_mentions(&state.db, body).await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat_messages (id, channel_id, author_id, body_md, body_html, reply_to_id, attachments, mentions) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(channel_id)
    .bind(user.id)
    .bind(body)
    .bind(&rendered.html)
    .bind(reply_to)
    .bind(&attachments)
    .bind(&rendered.mentioned)
    .execute(&state.db)
    .await?;
    sqlx::query("UPDATE chat_channels SET last_message_at = now() WHERE id = $1")
        .bind(channel_id)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE chat_members SET last_read_at = now(), hidden = FALSE WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    // Re-show hidden DMs for the recipients.
    if acc.kind.is_dm() {
        sqlx::query("UPDATE chat_members SET hidden = FALSE WHERE channel_id = $1")
            .bind(channel_id)
            .execute(&state.db)
            .await?;
    }

    let message = message_view(&state.db, user.id, id).await?;
    let mut broadcast = message.clone();
    broadcast.mentions_me = false;
    state
        .hub
        .publish(&Topic::Chat(channel_id), &ServerMsg::ChatMessage { message: broadcast });

    // Badges + notifications for members.
    let members: Vec<(Uuid, bool)> = sqlx::query_as("SELECT user_id, muted FROM chat_members WHERE channel_id = $1")
        .bind(channel_id)
        .fetch_all(&state.db)
        .await?;
    let excerpt = if body.trim().is_empty() {
        "📎 Anhang".to_string()
    } else {
        markdown::excerpt(body, 140)
    };
    for (member, muted) in members {
        if member == user.id {
            continue;
        }
        let mentioned = rendered.mentioned.contains(&member);
        state.hub.send_user(member, &ServerMsg::ChatActivity { channel_id, mention: mentioned });
        let watching = state.hub.user_in_topic(member, &Topic::Chat(channel_id));
        if watching {
            continue;
        }
        if acc.kind.is_dm() && !muted {
            notifications::notify(
                state,
                NewNotification {
                    user_id: member,
                    kind: NotificationKind::DirectMessage,
                    actor_id: Some(user.id),
                    title: format!("Nachricht von {}", user.display_name),
                    body: excerpt.clone(),
                    link: format!("/chat/{channel_id}?m={id}"),
                    subject: Some((SubjectType::ChatMessage, id)),
                },
            )
            .await;
        } else if mentioned {
            notifications::notify(
                state,
                NewNotification {
                    user_id: member,
                    kind: NotificationKind::Mention,
                    actor_id: Some(user.id),
                    title: format!("{} hat dich in #{} erwähnt", user.display_name, acc.name),
                    body: excerpt.clone(),
                    link: format!("/chat/{channel_id}?m={id}"),
                    subject: Some((SubjectType::ChatMessage, id)),
                },
            )
            .await;
        }
    }
    // Mentioned users who are not members of a public channel.
    if acc.kind == ChannelKind::Public {
        for uid in &rendered.mentioned {
            let member: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat_members WHERE channel_id = $1 AND user_id = $2)")
                .bind(channel_id)
                .bind(uid)
                .fetch_one(&state.db)
                .await?;
            if !member && *uid != user.id {
                notifications::notify(
                    state,
                    NewNotification {
                        user_id: *uid,
                        kind: NotificationKind::Mention,
                        actor_id: Some(user.id),
                        title: format!("{} hat dich in #{} erwähnt", user.display_name, acc.name),
                        body: excerpt.clone(),
                        link: format!("/chat/{channel_id}?m={id}"),
                        subject: Some((SubjectType::ChatMessage, id)),
                    },
                )
                .await;
            }
        }
    }
    Ok(message)
}

struct MessageMeta {
    channel_id: Uuid,
    author_id: Option<Uuid>,
    deleted: bool,
    body_md: String,
}

async fn message_meta(db: &PgPool, id: Uuid) -> Result<MessageMeta> {
    let r = sqlx::query("SELECT channel_id, author_id, deleted_at, body_md FROM chat_messages WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Nachricht"))?;
    let deleted: Option<DateTime<Utc>> = r.try_get("deleted_at")?;
    Ok(MessageMeta {
        channel_id: r.try_get("channel_id")?,
        author_id: r.try_get("author_id")?,
        deleted: deleted.is_some(),
        body_md: r.try_get("body_md")?,
    })
}

pub async fn edit(state: &AppState, user: &AuthUser, id: Uuid, body_md: &str) -> Result<ChatMessageView> {
    let meta = message_meta(&state.db, id).await?;
    readable(&state.db, user, meta.channel_id).await?;
    if meta.author_id != Some(user.id) {
        return Err(Error::Forbidden);
    }
    if meta.deleted {
        return Err(Error::invalid("Die Nachricht wurde gelöscht."));
    }
    let body = body_md.trim_end();
    if body.trim().is_empty() || body.chars().count() > MAX_MESSAGE_LEN {
        return Err(Error::invalid("Ungültige Nachricht."));
    }
    if body != meta.body_md {
        let rendered = markdown::render_mentions(&state.db, body).await?;
        sqlx::query("UPDATE chat_messages SET body_md = $2, body_html = $3, mentions = $4, edited_at = now() WHERE id = $1")
            .bind(id)
            .bind(body)
            .bind(&rendered.html)
            .bind(&rendered.mentioned)
            .execute(&state.db)
            .await?;
    }
    let message = message_view(&state.db, user.id, id).await?;
    state.hub.publish(
        &Topic::Chat(meta.channel_id),
        &ServerMsg::ChatMessageUpdated { message: message.clone() },
    );
    Ok(message)
}

pub async fn delete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let meta = message_meta(&state.db, id).await?;
    let acc = readable(&state.db, user, meta.channel_id).await?;
    let own = meta.author_id == Some(user.id);
    if !own && !acc.can_moderate(user) {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE chat_messages SET deleted_at = now(), pinned_at = NULL WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if !own {
        modlog::record(&state.db, Some(user.id), "delete_content", Some((SubjectType::ChatMessage, id)), meta.author_id, &markdown::excerpt(&meta.body_md, 120)).await?;
    }
    state.hub.publish(
        &Topic::Chat(meta.channel_id),
        &ServerMsg::ChatMessageDeleted {
            channel_id: meta.channel_id,
            message_id: id,
        },
    );
    Ok(())
}

pub async fn set_pinned(state: &AppState, user: &AuthUser, id: Uuid, pinned: bool) -> Result<ChatMessageView> {
    let meta = message_meta(&state.db, id).await?;
    let acc = readable(&state.db, user, meta.channel_id).await?;
    let allowed = acc.can_moderate(user) || (acc.kind.is_dm() && acc.is_member());
    if !allowed {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE chat_messages SET pinned_at = CASE WHEN $2 THEN now() END, pinned_by = CASE WHEN $2 THEN $3 END WHERE id = $1")
        .bind(id)
        .bind(pinned)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    let message = message_view(&state.db, user.id, id).await?;
    state.hub.publish(
        &Topic::Chat(meta.channel_id),
        &ServerMsg::ChatMessageUpdated { message: message.clone() },
    );
    Ok(message)
}

pub async fn pinned(db: &PgPool, user: &AuthUser, channel_id: Uuid) -> Result<Vec<ChatMessageView>> {
    readable(db, user, channel_id).await?;
    let rows = sqlx::query(&format!(
        "{MESSAGE_SELECT} WHERE m.channel_id = $1 AND m.pinned_at IS NOT NULL AND m.deleted_at IS NULL \
         ORDER BY m.pinned_at DESC LIMIT 50"
    ))
    .bind(channel_id)
    .fetch_all(db)
    .await?;
    let raw = rows.iter().map(|r| message_from(r, user.id)).collect::<Result<Vec<_>>>()?;
    finish_messages(db, raw, user.id).await
}

pub async fn search(db: &PgPool, user: &AuthUser, channel_id: Uuid, query: &str) -> Result<Vec<ChatMessageView>> {
    readable(db, user, channel_id).await?;
    let q = query.trim();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(&format!(
        "{MESSAGE_SELECT} WHERE m.channel_id = $1 AND m.deleted_at IS NULL \
           AND (m.search @@ websearch_to_tsquery('german', $2) OR position(lower($2) in lower(m.body_md)) > 0) \
         ORDER BY m.created_at DESC LIMIT 50"
    ))
    .bind(channel_id)
    .bind(q)
    .fetch_all(db)
    .await?;
    let raw = rows.iter().map(|r| message_from(r, user.id)).collect::<Result<Vec<_>>>()?;
    finish_messages(db, raw, user.id).await
}

pub async fn mark_read(db: &PgPool, user: &AuthUser, channel_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE chat_members SET last_read_at = now() WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(user.id)
        .execute(db)
        .await?;
    Ok(())
}

/// Attachment access for private channels / DMs (files with context).
pub async fn attachment_access(db: &PgPool, file_id: Uuid, channel_id: Uuid, viewer: Option<&AuthUser>) -> bool {
    let Some(v) = viewer else { return false };
    let used: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_messages WHERE channel_id = $1 AND $2 = ANY(attachments) AND deleted_at IS NULL)",
    )
    .bind(channel_id)
    .bind(file_id)
    .fetch_one(db)
    .await
    .unwrap_or(false);
    used && matches!(access(db, channel_id, v.id).await, Ok(Some(a)) if a.can_read(v))
}

/// Visibility for uploads attached to a channel.
pub async fn upload_visibility(db: &PgPool, user: &AuthUser, channel_id: Uuid) -> Result<Visibility> {
    let acc = readable(db, user, channel_id).await?;
    if !acc.can_post(user) {
        return Err(Error::Forbidden);
    }
    Ok(match acc.kind {
        ChannelKind::Public | ChannelKind::Room => Visibility::Members,
        _ => Visibility::Private,
    })
}

pub fn is_chat_upload(purpose: FilePurpose) -> bool {
    purpose == FilePurpose::Chat
}

/// Reaction changes are pushed to everyone in the channel.
pub async fn broadcast_reactions(state: &AppState, message_id: Uuid) -> Result<()> {
    let meta = message_meta(&state.db, message_id).await?;
    let mut counts = reactions::counts_one(&state.db, SubjectType::ChatMessage, message_id, None).await?;
    for c in &mut counts {
        c.me = false;
    }
    state.hub.publish(
        &Topic::Chat(meta.channel_id),
        &ServerMsg::ChatReactions {
            channel_id: meta.channel_id,
            message_id,
            reactions: counts,
        },
    );
    Ok(())
}

/// Subject info for reactions/reports on chat messages.
pub async fn subject_info(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<Option<crate::subjects::SubjectInfo>> {
    let Some(v) = viewer else { return Ok(None) };
    let row = sqlx::query("SELECT m.channel_id, m.author_id, m.body_md, m.deleted_at FROM chat_messages m WHERE m.id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?;
    let Some(r) = row else { return Ok(None) };
    let channel_id: Uuid = r.try_get("channel_id")?;
    let Some(acc) = access(db, channel_id, v.id).await? else { return Ok(None) };
    if !acc.can_read(v) {
        return Ok(None);
    }
    let body: String = r.try_get("body_md")?;
    Ok(Some(crate::subjects::SubjectInfo {
        author_id: r.try_get("author_id")?,
        title: if acc.kind.is_dm() { "Direktnachricht".into() } else { format!("#{}", acc.name) },
        preview: markdown::excerpt(&body, 200),
        link: Some(format!("/chat/{channel_id}?m={id}")),
    }))
}

pub async fn total_messages(db: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM chat_messages WHERE deleted_at IS NULL AND kind = 'text'")
        .fetch_one(db)
        .await?)
}

/// Markdown source of an own message (for editing).
pub async fn source(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<String> {
    let meta = message_meta(db, id).await?;
    readable(db, user, meta.channel_id).await?;
    if meta.author_id != Some(user.id) || meta.deleted {
        return Err(Error::Forbidden);
    }
    Ok(meta.body_md)
}
