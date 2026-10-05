//! Server-side logic of the community platform: database access, domain
//! services, the realtime hub and the HTTP routes that are not Leptos
//! server functions (WebSocket, uploads, downloads, feeds).

pub mod admin;
pub mod auth;
pub mod config;
pub mod error;
pub mod extensions;
pub mod forum;
pub mod http;
pub mod hub;
pub mod jobs;
mod jobs_ext;
pub mod mail;
pub mod maintenance;
mod maintenance_ext;
pub mod markdown;
pub mod modlog;
pub mod notifications;
pub mod ratelimit;
pub mod reactions;
pub mod realtime;
mod realtime_ext;
pub mod reports;
pub mod search;
mod search_ext;
pub mod settings;
pub mod state;
mod stats_ext;
pub mod subjects;
pub mod users;

use axum::extract::FromRef;
use axum::routing::get;
use axum::Router;

pub use config::Config;
pub use error::{Error, Result};
pub use state::AppState;

/// SQL expression for the bytes stored by the user in column `user_col`.
pub fn storage_usage_sql(_user_col: &str) -> String {
    "0::bigint".to_string()
}

pub async fn migrate(db: &sqlx::PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(db).await
}

/// Routes outside of Leptos (WebSocket, files, feeds, APIs).
pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AppState: FromRef<S>,
{
    Router::new().route("/ws", get(realtime::ws_handler))
}
