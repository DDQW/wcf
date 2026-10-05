use leptos::prelude::*;
use shared::AuthUser;
use tower_sessions::Session;
use uuid::Uuid;

use super::db::{users, DbPool};

pub const SESSION_USER_KEY: &str = "user_id";

/// Reads the logged-in user (if any) from the request's session cookie.
/// Every call re-checks the DB — never trust a stale value.
pub async fn current_user() -> Result<Option<AuthUser>, ServerFnError> {
    let session: Session = leptos_axum::extract().await?;
    let pool = use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool missing"))?;
    let user_id: Option<Uuid> = session
        .get(SESSION_USER_KEY)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    match user_id {
        Some(id) => Ok(users::find_by_id(&pool, id)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?),
        None => Ok(None),
    }
}

/// Same as [`current_user`] but rejects the request when not logged in.
/// Every mutating server function must call this before writing.
pub async fn require_user() -> Result<AuthUser, ServerFnError> {
    current_user()
        .await?
        .ok_or_else(|| ServerFnError::new("not authenticated"))
}

pub async fn log_in(session: &Session, user_id: Uuid) -> Result<(), ServerFnError> {
    session
        .insert(SESSION_USER_KEY, user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

pub async fn log_out(session: &Session) -> Result<(), ServerFnError> {
    session
        .flush()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
