use leptos::prelude::*;
use shared::AuthUser;

#[server]
pub async fn register(
    username: String,
    email: String,
    password: String,
    display_name: String,
) -> Result<AuthUser, ServerFnError> {
    use crate::ssr::db::{users, DbPool};
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    use argon2::Argon2;
    use tower_sessions::Session;

    if username.trim().len() < 3 {
        return Err(ServerFnError::new("username must be at least 3 characters"));
    }
    if password.len() < 8 {
        return Err(ServerFnError::new("password must be at least 8 characters"));
    }

    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .to_string();

    let is_first = users::is_first_user(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let role = if is_first { "admin" } else { "member" };

    let display_name = if display_name.trim().is_empty() {
        username.clone()
    } else {
        display_name
    };

    let user = users::create_user(&pool, &username, &email, &password_hash, &display_name, role)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let session: Session = leptos_axum::extract().await?;
    crate::ssr::auth::log_in(&session, user.id).await?;

    Ok(user)
}

#[server]
pub async fn login(username_or_email: String, password: String) -> Result<AuthUser, ServerFnError> {
    use crate::ssr::db::{users, DbPool};
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    use argon2::Argon2;
    use tower_sessions::Session;

    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;

    let (user_id, password_hash) = users::find_credentials(&pool, &username_or_email)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("invalid username/email or password"))?;

    let parsed_hash = PasswordHash::new(&password_hash)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| ServerFnError::new("invalid username/email or password"))?;

    let user = users::find_by_id(&pool, user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("invalid username/email or password"))?;

    let session: Session = leptos_axum::extract().await?;
    crate::ssr::auth::log_in(&session, user.id).await?;

    Ok(user)
}

#[server]
pub async fn logout() -> Result<(), ServerFnError> {
    use tower_sessions::Session;
    let session: Session = leptos_axum::extract().await?;
    crate::ssr::auth::log_out(&session).await
}

#[server]
pub async fn get_current_user() -> Result<Option<AuthUser>, ServerFnError> {
    crate::ssr::auth::current_user().await
}
