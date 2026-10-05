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
}
