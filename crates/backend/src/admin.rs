use shared::{AdminStats, AuthUser};
use sqlx::{PgPool, Row};

use crate::error::{Error, Result};
use crate::state::AppState;

pub async fn stats(state: &AppState, viewer: &AuthUser) -> Result<AdminStats> {
    if !viewer.is_mod() {
        return Err(Error::Forbidden);
    }
    let r = sqlx::query(&format!(
        "SELECT \
           (SELECT count(*) FROM users WHERE is_active) AS users, \
           (SELECT count(*) FROM users WHERE created_at > now() - interval '7 days') AS users_new_7d, \
           (SELECT count(*) FROM forum_topics WHERE NOT is_deleted) AS topics, \
           (SELECT count(*) FROM forum_posts WHERE NOT is_deleted) AS posts, \
           (SELECT count(*) FROM reports WHERE status = 'open') AS open_reports, \
           {} AS extra",
        crate::stats_ext::EXTRA_SQL
    ))
    .fetch_one(&state.db)
    .await?;
    let mut stats = AdminStats {
        users: r.try_get("users")?,
        users_new_7d: r.try_get("users_new_7d")?,
        online_now: state.hub.online_count() as i64,
        topics: r.try_get("topics")?,
        posts: r.try_get("posts")?,
        open_reports: r.try_get("open_reports")?,
        ..Default::default()
    };
    crate::stats_ext::fill(&state.db, &mut stats).await?;
    Ok(stats)
}

/// Members, topics and posts for the public start page.
pub async fn community_counts(db: &PgPool) -> Result<(i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT (SELECT count(*) FROM users WHERE is_active),                 (SELECT count(*) FROM forum_topics WHERE NOT is_deleted),                 (SELECT count(*) FROM forum_posts WHERE NOT is_deleted)",
    )
    .fetch_one(db)
    .await?)
}
