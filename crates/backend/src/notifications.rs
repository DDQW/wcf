use chrono::{DateTime, Utc};
use shared::ws::ServerMsg;
use shared::{Cursor, NotificationKind, NotificationView, Page, SubjectType};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::Result;
use crate::state::AppState;
use crate::users::{self, opt_user, user_cols};

pub struct NewNotification {
    pub user_id: Uuid,
    pub kind: NotificationKind,
    pub actor_id: Option<Uuid>,
    pub title: String,
    pub body: String,
    pub link: String,
    pub subject: Option<(SubjectType, Uuid)>,
}

/// Stores a notification and pushes it to the user's open connections.
/// Skips self-notifications and kinds the user switched off. Errors are
/// logged rather than propagated: a failed notification must never fail
/// the action that triggered it.
pub async fn notify(state: &AppState, n: NewNotification) {
    if let Err(e) = try_notify(state, n).await {
        tracing::warn!(error = %e, "notification failed");
    }
}

async fn try_notify(state: &AppState, n: NewNotification) -> Result<()> {
    if n.actor_id == Some(n.user_id) {
        return Ok(());
    }
    let prefs = users::notify_prefs(&state.db, n.user_id).await?;
    if !n.kind.enabled_in(&prefs) {
        return Ok(());
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO notifications (id, user_id, kind, actor_id, title, body, link, subject_type, subject_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(n.user_id)
    .bind(n.kind.as_str())
    .bind(n.actor_id)
    .bind(&n.title)
    .bind(&n.body)
    .bind(&n.link)
    .bind(n.subject.map(|(t, _)| t.as_str()))
    .bind(n.subject.map(|(_, id)| id))
    .execute(&state.db)
    .await?;
    if state.hub.is_online(n.user_id) {
        let actor = match n.actor_id {
            Some(a) => users::summary(&state.db, a).await?,
            None => None,
        };
        let unread = unread_count(&state.db, n.user_id).await?;
        state.hub.send_user(
            n.user_id,
            &ServerMsg::Notification {
                notification: NotificationView {
                    id,
                    kind: n.kind,
                    actor,
                    title: n.title,
                    body: n.body,
                    link: n.link,
                    is_read: false,
                    created_at: Utc::now(),
                },
                unread,
            },
        );
    }
    Ok(())
}

pub async fn unread_count(db: &PgPool, user_id: Uuid) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1 AND NOT is_read")
            .bind(user_id)
            .fetch_one(db)
            .await?,
    )
}

pub async fn list(
    db: &PgPool,
    user_id: Uuid,
    cursor: Option<Cursor>,
    unread_only: bool,
    limit: i64,
) -> Result<Page<NotificationView>> {
    let rows = sqlx::query(&format!(
        "SELECT n.id, n.kind, n.title, n.body, n.link, n.is_read, n.created_at, {} \
         FROM notifications n LEFT JOIN users a ON a.id = n.actor_id \
         WHERE n.user_id = $1 \
           AND ($2::timestamptz IS NULL OR (n.created_at, n.id) < ($2, $3)) \
           AND (NOT $4 OR NOT n.is_read) \
         ORDER BY n.created_at DESC, n.id DESC LIMIT $5",
        user_cols("a", "actor")
    ))
    .bind(user_id)
    .bind(cursor.map(|c| c.ts))
    .bind(cursor.map(|c| c.id))
    .bind(unread_only)
    .bind(limit + 1)
    .fetch_all(db)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        let kind: String = r.try_get("kind")?;
        items.push(NotificationView {
            id: r.try_get("id")?,
            kind: NotificationKind::parse(&kind),
            actor: opt_user(r, "actor")?,
            title: r.try_get("title")?,
            body: r.try_get("body")?,
            link: r.try_get("link")?,
            is_read: r.try_get("is_read")?,
            created_at: r.try_get::<DateTime<Utc>, _>("created_at")?,
        });
    }
    let next = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items.last().map(|n| Cursor::new(n.created_at, n.id).encode())
    } else {
        None
    };
    Ok(Page::new(items, next))
}

async fn push_read_state(state: &AppState, user_id: Uuid) -> Result<i64> {
    let unread = unread_count(&state.db, user_id).await?;
    state
        .hub
        .send_user(user_id, &ServerMsg::NotificationsRead { unread });
    Ok(unread)
}

pub async fn mark_read(state: &AppState, user_id: Uuid, ids: &[Uuid]) -> Result<i64> {
    sqlx::query("UPDATE notifications SET is_read = TRUE WHERE user_id = $1 AND id = ANY($2)")
        .bind(user_id)
        .bind(ids)
        .execute(&state.db)
        .await?;
    push_read_state(state, user_id).await
}

pub async fn mark_all_read(state: &AppState, user_id: Uuid) -> Result<i64> {
    sqlx::query("UPDATE notifications SET is_read = TRUE WHERE user_id = $1 AND NOT is_read")
        .bind(user_id)
        .execute(&state.db)
        .await?;
    push_read_state(state, user_id).await
}

/// Marks notifications pointing at a link as read (e.g. when the user
/// opens the topic the notification is about).
pub async fn mark_link_read(state: &AppState, user_id: Uuid, link_prefix: &str) -> Result<()> {
    let res = sqlx::query(
        "UPDATE notifications SET is_read = TRUE \
         WHERE user_id = $1 AND NOT is_read AND starts_with(link, $2)",
    )
    .bind(user_id)
    .bind(link_prefix)
    .execute(&state.db)
    .await?;
    if res.rows_affected() > 0 {
        push_read_state(state, user_id).await?;
    }
    Ok(())
}

pub async fn delete_all(state: &AppState, user_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM notifications WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await?;
    push_read_state(state, user_id).await?;
    Ok(())
}

pub async fn purge_old(db: &PgPool) -> Result<()> {
    sqlx::query("DELETE FROM notifications WHERE is_read AND created_at < now() - interval '90 days'")
        .execute(db)
        .await?;
    Ok(())
}
