//! Background jobs of modules added in later phases.

use crate::state::AppState;

pub async fn every_minute(_state: &AppState) {}

pub async fn hourly(state: &AppState) {
    if let Err(e) = crate::files::purge_old_trash(state).await {
        tracing::warn!(error = %e, "trash purge failed");
    }
    state.storage.clean_tmp().await;
}
