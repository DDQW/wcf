//! Server-side logic of the community platform: database access, domain
//! services, the realtime hub and the HTTP routes that are not Leptos
//! server functions (WebSocket, uploads, downloads, feeds).

pub mod admin;
pub mod auth;
pub mod blogs;
pub mod chat;
pub mod cms;
pub mod comments;
pub mod config;
pub mod error;
pub mod extensions;
pub mod files;
pub mod files_http;
pub mod forum;
pub mod http;
pub mod hub;
pub mod jobs;
mod jobs_ext;
pub mod mail;
pub mod maintenance;
mod maintenance_ext;
pub mod markdown;
pub mod media;
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
pub mod storage;
mod stats_ext;
pub mod subjects;
pub mod users;

use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;

pub use config::Config;
pub use error::{Error, Result};
pub use state::AppState;

/// SQL expression for the bytes stored by the user in column `user_col`.
pub fn storage_usage_sql(user_col: &str) -> String {
    format!("(SELECT coalesce(sum(size_bytes), 0)::bigint FROM files WHERE owner_id = {user_col})")
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
    Router::new()
        .route("/ws", get(realtime::ws_handler))
        .route(
            "/api/files",
            post(files_http::upload).layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route("/files/{id}", get(files_http::serve_inline))
        .route("/files/{id}/download", get(files_http::serve_download))
        .route("/files/{id}/thumb", get(files_http::serve_thumb))
        .route("/files/{id}/preview", get(files_http::serve_preview))
        .route("/blogs/feed.xml", get(blogs::rss_all_handler))
        .route("/api/cms/{type_slug}", get(cms::api_list))
        .route("/api/cms/{type_slug}/{slug}", get(cms::api_get))
        .route("/blog/{username}/feed.xml", get(blogs::rss_blog_handler))
}
