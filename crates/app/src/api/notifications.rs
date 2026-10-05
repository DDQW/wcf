use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::{NotificationView, Page};
use uuid::Uuid;

use crate::error::AppError;

#[server(input = Json)]
pub async fn list_notifications(
    cursor: Option<String>,
    unread_only: bool,
) -> Result<Page<NotificationView>, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    let cursor = cursor.and_then(|c| shared::Cursor::decode(&c).ok());
    Ok(backend::notifications::list(&st.db, user.id, cursor, unread_only, 20).await?)
}

#[server(input = Json)]
pub async fn mark_notifications_read(ids: Vec<Uuid>) -> Result<i64, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::notifications::mark_read(&st, user.id, &ids).await?)
}

#[server(input = Json)]
pub async fn mark_all_notifications_read() -> Result<i64, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::notifications::mark_all_read(&st, user.id).await?)
}

#[server(input = Json)]
pub async fn clear_notifications() -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    backend::notifications::delete_all(&st, user.id).await?;
    Ok(())
}
