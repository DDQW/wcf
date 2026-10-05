//! Periodic background maintenance.

use std::time::Duration;

use crate::state::AppState;
use crate::{auth, notifications};

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        let mut minutes: u64 = 0;
        loop {
            tick.tick().await;
            minutes += 1;
            crate::jobs_ext::every_minute(&state).await;
            if minutes % 10 == 0 {
                state.limiter.cleanup(Duration::from_secs(3600));
            }
            if minutes % 60 == 0 {
                if let Err(e) = auth::purge_expired_resets(&state.db).await {
                    tracing::warn!(error = %e, "purge resets failed");
                }
                if let Err(e) = notifications::purge_old(&state.db).await {
                    tracing::warn!(error = %e, "purge notifications failed");
                }
                crate::jobs_ext::hourly(&state).await;
            }
        }
    });
}
