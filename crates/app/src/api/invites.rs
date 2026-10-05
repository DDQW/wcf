use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::admin::{InviteInput, InviteView};

use crate::error::AppError;

#[server(input = Json)]
pub async fn list_invites() -> Result<Vec<InviteView>, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::invites::list(&st, &user).await?)
}

#[server(input = Json)]
pub async fn create_invites(input: InviteInput) -> Result<Vec<InviteView>, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::invites::create(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn revoke_invite(code: String) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::invites::revoke(&st, &user, &code).await?)
}
