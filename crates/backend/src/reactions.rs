use std::collections::HashMap;

use shared::reaction::is_valid_reaction;
use shared::{AuthUser, NotificationKind, ReactionCount, SubjectType, UserSummary};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::subjects;
use crate::users::{user_cols, user_from};

/// Aggregated reactions for many subjects of one type.
pub async fn counts_for(
    db: &PgPool,
    st: SubjectType,
    ids: &[Uuid],
    viewer: Option<Uuid>,
) -> Result<HashMap<Uuid, Vec<ReactionCount>>> {
    let mut map: HashMap<Uuid, Vec<ReactionCount>> = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let rows = sqlx::query(
        "SELECT subject_id, emoji, count(*) AS n, bool_or(user_id = $3) AS me, min(created_at) AS first \
         FROM reactions WHERE subject_type = $1 AND subject_id = ANY($2) \
         GROUP BY subject_id, emoji ORDER BY first",
    )
    .bind(st.as_str())
    .bind(ids)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    for r in rows {
        let id: Uuid = r.try_get("subject_id")?;
        map.entry(id).or_default().push(ReactionCount {
            emoji: r.try_get("emoji")?,
            count: r.try_get("n")?,
            me: r.try_get::<Option<bool>, _>("me")?.unwrap_or(false),
        });
    }
    Ok(map)
}

pub async fn counts_one(db: &PgPool, st: SubjectType, id: Uuid, viewer: Option<Uuid>) -> Result<Vec<ReactionCount>> {
    Ok(counts_for(db, st, &[id], viewer).await?.remove(&id).unwrap_or_default())
}

/// Adds or removes the viewer's reaction and returns the new counts.
pub async fn toggle(
    state: &AppState,
    user: &AuthUser,
    st: SubjectType,
    id: Uuid,
    emoji: &str,
) -> Result<Vec<ReactionCount>> {
    if !is_valid_reaction(emoji) {
        return Err(Error::invalid("Diese Reaktion gibt es nicht."));
    }
    state
        .limiter
        .hit(&format!("react:{}", user.id), 60, std::time::Duration::from_secs(60))?;
    let info = subjects::lookup(state, Some(user), st, id)
        .await?
        .ok_or(Error::NotFound("Inhalt"))?;

    let removed = sqlx::query(
        "DELETE FROM reactions WHERE subject_type = $1 AND subject_id = $2 AND user_id = $3 AND emoji = $4",
    )
    .bind(st.as_str())
    .bind(id)
    .bind(user.id)
    .bind(emoji)
    .execute(&state.db)
    .await?
    .rows_affected()
        > 0;

    if !removed {
        sqlx::query(
            "INSERT INTO reactions (subject_type, subject_id, user_id, emoji) VALUES ($1, $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .bind(st.as_str())
        .bind(id)
        .bind(user.id)
        .bind(emoji)
        .execute(&state.db)
        .await?;
        if let (Some(author), Some(link)) = (info.author_id, info.link.clone()) {
            notifications::notify(
                state,
                NewNotification {
                    user_id: author,
                    kind: NotificationKind::Reaction,
                    actor_id: Some(user.id),
                    title: format!("{} hat mit {emoji} reagiert", user.display_name),
                    body: info.preview.clone(),
                    link,
                    subject: Some((st, id)),
                },
            )
            .await;
        }
    }
    crate::extensions::after_reaction(state, st, id).await;
    counts_one(&state.db, st, id, Some(user.id)).await
}

/// Users who reacted with a given emoji (for tooltips).
pub async fn reactors(db: &PgPool, st: SubjectType, id: Uuid, emoji: &str) -> Result<Vec<UserSummary>> {
    let rows = sqlx::query(&format!(
        "SELECT {} FROM reactions r JOIN users u ON u.id = r.user_id \
         WHERE r.subject_type = $1 AND r.subject_id = $2 AND r.emoji = $3 \
         ORDER BY r.created_at LIMIT 50",
        user_cols("u", "u")
    ))
    .bind(st.as_str())
    .bind(id)
    .bind(emoji)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(|r| user_from(r, "u")).collect::<Result<_, _>>()?)
}

pub async fn delete_for(db: &PgPool, st: SubjectType, ids: &[Uuid]) -> Result<()> {
    sqlx::query("DELETE FROM reactions WHERE subject_type = $1 AND subject_id = ANY($2)")
        .bind(st.as_str())
        .bind(ids)
        .execute(db)
        .await?;
    Ok(())
}
