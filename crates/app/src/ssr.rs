//! Server-side helpers for server functions (never compiled to WASM).

use backend::AppState;
use leptos::prelude::*;
use shared::{AuthUser, ModuleToggles};
use tower_sessions::Session;

use crate::error::AppError;

pub fn state() -> Result<AppState, AppError> {
    use_context::<AppState>().ok_or_else(|| AppError::Internal("Serverzustand fehlt.".into()))
}

pub async fn session() -> Result<Session, AppError> {
    leptos_axum::extract::<Session>()
        .await
        .map_err(|e| AppError::Internal(format!("Session: {e}")))
}

/// The logged-in user of the current request, if any (re-read from the DB).
pub async fn viewer() -> Result<Option<AuthUser>, AppError> {
    let st = state()?;
    let session = session().await?;
    Ok(backend::auth::user_from_session(&st.db, &session).await?)
}

pub async fn require_user() -> Result<AuthUser, AppError> {
    viewer().await?.ok_or(AppError::Unauthorized)
}

pub async fn require_mod() -> Result<AuthUser, AppError> {
    let u = require_user().await?;
    if u.is_mod() {
        Ok(u)
    } else {
        Err(AppError::Forbidden)
    }
}

pub async fn require_admin() -> Result<AuthUser, AppError> {
    let u = require_user().await?;
    if u.is_admin() {
        Ok(u)
    } else {
        Err(AppError::Forbidden)
    }
}

/// Fails if an admin switched the given module off.
pub fn require_module(check: impl Fn(&ModuleToggles) -> bool) -> Result<AppState, AppError> {
    let st = state()?;
    if check(&st.settings().modules) {
        Ok(st)
    } else {
        Err(AppError::NotFound("Dieser Bereich ist deaktiviert.".into()))
    }
}

pub async fn client_ip() -> String {
    use axum::extract::ConnectInfo;
    use axum::http::HeaderMap;
    use std::net::SocketAddr;
    let trust = state().map(|s| s.config.trust_proxy).unwrap_or(false);
    let headers: HeaderMap = leptos_axum::extract().await.unwrap_or_default();
    let peer: Option<ConnectInfo<SocketAddr>> = leptos_axum::extract().await.ok();
    backend::http::client_ip(&headers, peer.map(|c| c.0), trust)
}

/// Sets the HTTP status of the current SSR response (e.g. 404 pages).
pub fn set_status(code: u16) {
    if let Some(res) = use_context::<leptos_axum::ResponseOptions>() {
        if let Ok(status) = axum::http::StatusCode::from_u16(code) {
            res.set_status(status);
        }
    }
}
