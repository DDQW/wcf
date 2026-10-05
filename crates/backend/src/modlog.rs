use shared::moderation::ModLogEntry;
use shared::SubjectType;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::Result;
use crate::users::{opt_user, user_cols};

/// Appends an entry to the moderation/audit log.
pub async fn record(
    db: &PgPool,
    actor: Option<Uuid>,
    action: &str,
    subject: Option<(SubjectType, Uuid)>,
    target_user: Option<Uuid>,
    details: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO mod_log (id, actor_id, action, subject_type, subject_id, target_user_id, details) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(actor)
    .bind(action)
    .bind(subject.map(|(t, _)| t.as_str()))
    .bind(subject.map(|(_, id)| id))
    .bind(target_user)
    .bind(details)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn list(db: &PgPool, offset: i64, limit: i64) -> Result<(Vec<ModLogEntry>, i64)> {
    let rows = sqlx::query(&format!(
        "SELECT l.id, l.action, l.subject_type, l.subject_id, l.details, l.created_at, {}, {} \
         FROM mod_log l \
         LEFT JOIN users a ON a.id = l.actor_id \
         LEFT JOIN users t ON t.id = l.target_user_id \
         ORDER BY l.created_at DESC, l.id DESC LIMIT $1 OFFSET $2",
        user_cols("a", "actor"),
        user_cols("t", "target"),
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM mod_log")
        .fetch_one(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(ModLogEntry {
            id: r.try_get("id")?,
            actor: opt_user(&r, "actor")?,
            action: r.try_get("action")?,
            subject_type: r.try_get("subject_type")?,
            subject_id: r.try_get("subject_id")?,
            target_user: opt_user(&r, "target")?,
            details: r.try_get("details")?,
            created_at: r.try_get("created_at")?,
        });
    }
    Ok((out, total))
}
