use std::time::Duration;

use shared::moderation::{ReportReason, ReportStatus, ReportView};
use shared::{AuthUser, NotificationKind, SubjectType};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::users::{opt_user, user_cols, user_from};
use crate::{modlog, subjects};

pub async fn create(
    state: &AppState,
    reporter: &AuthUser,
    st: SubjectType,
    id: Uuid,
    reason: ReportReason,
    details: &str,
) -> Result<()> {
    state
        .limiter
        .hit(&format!("report:{}", reporter.id), 10, Duration::from_secs(3600))?;
    if details.chars().count() > 2000 {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 2000 Zeichen)."));
    }
    let info = subjects::lookup(state, Some(reporter), st, id)
        .await?
        .ok_or(Error::NotFound("Inhalt"))?;
    if info.author_id == Some(reporter.id) {
        return Err(Error::invalid("Eigene Inhalte kannst du nicht melden."));
    }
    let res = sqlx::query(
        "INSERT INTO reports (id, reporter_id, subject_type, subject_id, reason, details) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(reporter.id)
    .bind(st.as_str())
    .bind(id)
    .bind(reason.as_str())
    .bind(details.trim())
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        let e = Error::from(e);
        if e.is_unique_violation(None) {
            return Err(Error::invalid("Du hast diesen Inhalt bereits gemeldet."));
        }
        return Err(e);
    }
    // Let online staff know there is something to review.
    let staff: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role IN ('admin', 'moderator') AND is_active",
    )
    .fetch_all(&state.db)
    .await?;
    for uid in staff {
        notifications::notify(
            state,
            NewNotification {
                user_id: uid,
                kind: NotificationKind::Moderation,
                actor_id: Some(reporter.id),
                title: format!("Neue Meldung: {} ({})", st.label(), reason.label()),
                body: info.preview.clone(),
                link: "/admin/reports".into(),
                subject: Some((st, id)),
            },
        )
        .await;
    }
    Ok(())
}

pub async fn open_count(db: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM reports WHERE status = 'open'")
        .fetch_one(db)
        .await?)
}

pub async fn list(
    state: &AppState,
    viewer: &AuthUser,
    status: ReportStatus,
    offset: i64,
    limit: i64,
) -> Result<(Vec<ReportView>, i64)> {
    if !viewer.is_mod() {
        return Err(Error::Forbidden);
    }
    let rows = sqlx::query(&format!(
        "SELECT r.id, r.subject_type, r.subject_id, r.reason, r.details, r.status, r.resolution_note, \
                r.created_at, r.resolved_at, {}, {} \
         FROM reports r JOIN users rp ON rp.id = r.reporter_id \
         LEFT JOIN users rs ON rs.id = r.resolved_by \
         WHERE r.status = $1 ORDER BY r.created_at DESC LIMIT $2 OFFSET $3",
        user_cols("rp", "reporter"),
        user_cols("rs", "resolver"),
    ))
    .bind(status.as_str())
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM reports WHERE status = $1")
        .bind(status.as_str())
        .fetch_one(&state.db)
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let st_raw: String = r.try_get("subject_type")?;
        let Some(st) = SubjectType::parse(&st_raw) else { continue };
        let subject_id: Uuid = r.try_get("subject_id")?;
        let info = subjects::lookup(state, Some(viewer), st, subject_id).await?;
        let content_author = match info.as_ref().and_then(|i| i.author_id) {
            Some(a) => crate::users::summary(&state.db, a).await?,
            None => None,
        };
        let reason: String = r.try_get("reason")?;
        let status: String = r.try_get("status")?;
        out.push(ReportView {
            id: r.try_get("id")?,
            subject_type: st,
            subject_id,
            preview: info.as_ref().map(|i| i.preview.clone()).unwrap_or_default(),
            link: info.as_ref().and_then(|i| i.link.clone()),
            content_author,
            reason: ReportReason::parse(&reason),
            details: r.try_get("details")?,
            reporter: user_from(&r, "reporter")?,
            status: ReportStatus::parse(&status),
            resolved_by: opt_user(&r, "resolver")?,
            resolution_note: r.try_get("resolution_note")?,
            created_at: r.try_get("created_at")?,
            resolved_at: r.try_get("resolved_at")?,
        });
    }
    Ok((out, total))
}

/// Closes a report. With `delete_content` the reported subject is removed
/// as well (and every other open report on it is closed too).
pub async fn resolve(
    state: &AppState,
    actor: &AuthUser,
    report_id: Uuid,
    status: ReportStatus,
    note: &str,
    delete_content: bool,
) -> Result<()> {
    if !actor.is_mod() {
        return Err(Error::Forbidden);
    }
    if status == ReportStatus::Open {
        return Err(Error::invalid("Ungültiger Status."));
    }
    let row: Option<(String, Uuid, Uuid)> = sqlx::query_as(
        "SELECT subject_type, subject_id, reporter_id FROM reports WHERE id = $1",
    )
    .bind(report_id)
    .fetch_optional(&state.db)
    .await?;
    let (st_raw, subject_id, reporter_id) = row.ok_or(Error::NotFound("Meldung"))?;
    let st = SubjectType::parse(&st_raw).ok_or(Error::NotFound("Meldung"))?;

    if delete_content {
        subjects::moderator_delete(state, actor, st, subject_id).await?;
    }
    let note = note.trim();
    let closed: Vec<Uuid> = if delete_content {
        sqlx::query_scalar(
            "UPDATE reports SET status = $1, resolved_by = $2, resolution_note = NULLIF($3, ''), resolved_at = now() \
             WHERE status = 'open' AND subject_type = $4 AND subject_id = $5 RETURNING reporter_id",
        )
        .bind(status.as_str())
        .bind(actor.id)
        .bind(note)
        .bind(st.as_str())
        .bind(subject_id)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query(
            "UPDATE reports SET status = $1, resolved_by = $2, resolution_note = NULLIF($3, ''), resolved_at = now() \
             WHERE id = $4",
        )
        .bind(status.as_str())
        .bind(actor.id)
        .bind(note)
        .bind(report_id)
        .execute(&state.db)
        .await?;
        vec![reporter_id]
    };
    modlog::record(
        &state.db,
        Some(actor.id),
        if status == ReportStatus::Resolved { "report_resolved" } else { "report_dismissed" },
        Some((st, subject_id)),
        None,
        note,
    )
    .await?;
    for reporter in closed {
        notifications::notify(
            state,
            NewNotification {
                user_id: reporter,
                kind: NotificationKind::Moderation,
                actor_id: None,
                title: match status {
                    ReportStatus::Resolved => "Deine Meldung wurde bearbeitet – danke!".into(),
                    _ => "Deine Meldung wurde geprüft und verworfen.".into(),
                },
                body: note.to_string(),
                link: String::new(),
                subject: Some((st, subject_id)),
            },
        )
        .await;
    }
    Ok(())
}
