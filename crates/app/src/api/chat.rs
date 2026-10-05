use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::chat::{ChannelDetail, ChannelView, ChatMessageView, ChatRole, MessagePage, NewChannel};
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
async fn chat_user() -> Result<(backend::AppState, shared::AuthUser), AppError> {
    let st = crate::ssr::require_module(|m| m.chat)?;
    let user = crate::ssr::require_user().await?;
    Ok((st, user))
}

#[server(input = Json)]
pub async fn my_channels() -> Result<Vec<ChannelView>, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::my_channels(&st.db, &user).await?)
}

#[server(input = Json)]
pub async fn browse_channels() -> Result<Vec<ChannelView>, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::browse(&st.db, &user).await?)
}

#[server(input = Json)]
pub async fn get_channel_detail(channel_id: Uuid) -> Result<ChannelDetail, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::detail(&st.db, &user, channel_id).await?)
}

#[server(input = Json)]
pub async fn list_messages(
    channel_id: Uuid,
    before: Option<String>,
    after: Option<String>,
    around: Option<Uuid>,
) -> Result<MessagePage, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::list_messages(&st.db, &user, channel_id, before, after, around).await?)
}

#[server(input = Json)]
pub async fn send_message(
    channel_id: Uuid,
    body: String,
    reply_to: Option<Uuid>,
    attachments: Vec<Uuid>,
) -> Result<ChatMessageView, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::send(&st, &user, channel_id, &body, reply_to, attachments).await?)
}

#[server(input = Json)]
pub async fn edit_message(message_id: Uuid, body: String) -> Result<ChatMessageView, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::edit(&st, &user, message_id, &body).await?)
}

#[server(input = Json)]
pub async fn message_source(message_id: Uuid) -> Result<String, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::source(&st.db, &user, message_id).await?)
}

#[server(input = Json)]
pub async fn delete_message(message_id: Uuid) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::delete(&st, &user, message_id).await?)
}

#[server(input = Json)]
pub async fn pin_message(message_id: Uuid, pinned: bool) -> Result<ChatMessageView, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::set_pinned(&st, &user, message_id, pinned).await?)
}

#[server(input = Json)]
pub async fn pinned_messages(channel_id: Uuid) -> Result<Vec<ChatMessageView>, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::pinned(&st.db, &user, channel_id).await?)
}

#[server(input = Json)]
pub async fn search_messages(channel_id: Uuid, query: String) -> Result<Vec<ChatMessageView>, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::search(&st.db, &user, channel_id, &query).await?)
}

#[server(input = Json)]
pub async fn mark_channel_read(channel_id: Uuid) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::mark_read(&st.db, &user, channel_id).await?)
}

#[server(input = Json)]
pub async fn create_channel(input: NewChannel) -> Result<Uuid, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::create_channel(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn update_channel(channel_id: Uuid, name: String, topic: String) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::update_channel(&st, &user, channel_id, &name, &topic).await?)
}

#[server(input = Json)]
pub async fn archive_channel(channel_id: Uuid, archived: bool) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::set_archived(&st, &user, channel_id, archived).await?)
}

#[server(input = Json)]
pub async fn join_channel(channel_id: Uuid) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::join(&st, &user, channel_id).await?)
}

#[server(input = Json)]
pub async fn leave_channel(channel_id: Uuid) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::leave(&st, &user, channel_id).await?)
}

#[server(input = Json)]
pub async fn add_channel_members(channel_id: Uuid, users: Vec<Uuid>) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::add_members(&st, &user, channel_id, &users).await?)
}

#[server(input = Json)]
pub async fn remove_channel_member(channel_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::remove_member(&st, &user, channel_id, user_id).await?)
}

#[server(input = Json)]
pub async fn set_channel_member_role(channel_id: Uuid, user_id: Uuid, role: ChatRole) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::set_member_role(&st, &user, channel_id, user_id, role).await?)
}

#[server(input = Json)]
pub async fn mute_channel(channel_id: Uuid, muted: bool) -> Result<(), AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::set_muted(&st.db, &user, channel_id, muted).await?)
}

#[server(input = Json)]
pub async fn open_dm(user_id: Uuid) -> Result<Uuid, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::open_dm(&st, &user, user_id).await?)
}

#[server(input = Json)]
pub async fn create_group(users: Vec<Uuid>, name: String) -> Result<Uuid, AppError> {
    let (st, user) = chat_user().await?;
    Ok(backend::chat::create_group(&st, &user, &users, &name).await?)
}

#[server(input = Json)]
pub async fn chat_unread_total() -> Result<i64, AppError> {
    let st = crate::ssr::state()?;
    match crate::ssr::viewer().await? {
        Some(user) if st.settings().modules.chat => Ok(backend::chat::unread_total(&st.db, &user).await?),
        _ => Ok(0),
    }
}
