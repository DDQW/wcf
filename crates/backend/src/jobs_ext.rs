//! Background jobs of modules added in later phases.

use crate::state::AppState;

pub async fn every_minute(state: &AppState) {
    if let Err(e) = crate::blogs::publish_due(state).await {
        tracing::warn!(error = %e, "scheduled blog posts failed");
    }
    if let Err(e) = crate::calls::cleanup_rooms(state).await {
        tracing::warn!(error = %e, "call room cleanup failed");
    }
}

pub async fn hourly(state: &AppState) {
    if let Err(e) = crate::files::purge_old_trash(state).await {
        tracing::warn!(error = %e, "trash purge failed");
    }
    state.storage.clean_tmp().await;
    // Share links that ran out a while ago (kept briefly so owners see why).
    if let Err(e) = sqlx::query(
        "DELETE FROM file_shares WHERE (expires_at < now() - interval '30 days')          OR (max_downloads IS NOT NULL AND download_count >= max_downloads AND created_at < now() - interval '30 days')",
    )
    .execute(&state.db)
    .await
    {
        tracing::warn!(error = %e, "share cleanup failed");
    }
    // Finished media jobs are only interesting for a while.
    if let Err(e) = sqlx::query(
        "DELETE FROM watch_jobs WHERE status IN ('done', 'error') AND updated_at < now() - interval '30 days'",
    )
    .execute(&state.db)
    .await
    {
        tracing::warn!(error = %e, "media job cleanup failed");
    }
}
