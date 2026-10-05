use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::Bootstrap;

use crate::error::AppError;

#[cfg(feature = "ssr")]
pub(crate) async fn build_bootstrap(
    st: &backend::AppState,
    user: Option<shared::AuthUser>,
) -> Result<Bootstrap, AppError> {
    let unread = match &user {
        Some(u) => backend::notifications::unread_count(&st.db, u.id).await?,
        None => 0,
    };
    Ok(Bootstrap {
        user,
        site: st.site_info(),
        unread_notifications: unread,
        nav: crate::api::nav_links(st).await,
    })
}

/// Session + site configuration for the app shell.
#[server(input = Json)]
pub async fn load_bootstrap() -> Result<Bootstrap, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::viewer().await?;
    build_bootstrap(&st, user).await
}

#[server(input = Json)]
pub async fn login(username_or_email: String, password: String) -> Result<Bootstrap, AppError> {
    let st = crate::ssr::state()?;
    let ip = crate::ssr::client_ip().await;
    let user = backend::auth::login(&st, &ip, &username_or_email, &password).await?;
    let session = crate::ssr::session().await?;
    backend::auth::start_session(&session, user.id).await?;
    build_bootstrap(&st, Some(user)).await
}

#[server(input = Json)]
pub async fn register(
    username: String,
    email: String,
    password: String,
    display_name: String,
) -> Result<Bootstrap, AppError> {
    let st = crate::ssr::state()?;
    let ip = crate::ssr::client_ip().await;
    let user = backend::auth::register(
        &st,
        &ip,
        backend::auth::RegisterInput {
            username,
            email,
            password,
            display_name,
        },
    )
    .await?;
    let session = crate::ssr::session().await?;
    backend::auth::start_session(&session, user.id).await?;
    build_bootstrap(&st, Some(user)).await
}

#[server(input = Json)]
pub async fn logout() -> Result<(), AppError> {
    let session = crate::ssr::session().await?;
    backend::auth::end_session(&session).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn request_password_reset(email: String) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let ip = crate::ssr::client_ip().await;
    backend::auth::request_reset(&st, &ip, &email).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn reset_password(token: String, password: String) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    backend::auth::reset_password(&st.db, &token, &password).await?;
    Ok(())
}
