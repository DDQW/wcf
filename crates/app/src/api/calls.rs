use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::call::{
    CallRoomDetail, CallRoomInput, CallRoomView, CallsOverview, DirectCallDetail, DirectCallView, JoinInfo,
};
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
async fn call_user() -> Result<(backend::AppState, shared::AuthUser), AppError> {
    let st = crate::ssr::require_module(|m| m.calls)?;
    let user = crate::ssr::require_user().await?;
    Ok((st, user))
}

#[server(input = Json)]
pub async fn fetch_calls_overview() -> Result<CallsOverview, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::overview(&st, &user).await?)
}

#[server(input = Json)]
pub async fn get_call_room(id: Uuid) -> Result<CallRoomDetail, AppError> {
    let (st, user) = call_user().await?;
    let res = backend::calls::room_detail(&st, &user, id).await;
    if matches!(res, Err(backend::Error::NotFound(_))) {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn create_call_room(input: CallRoomInput) -> Result<CallRoomView, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::create_room(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn update_call_room(id: Uuid, input: CallRoomInput) -> Result<CallRoomView, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::update_room(&st, &user, id, input).await?)
}

#[server(input = Json)]
pub async fn delete_call_room(id: Uuid) -> Result<(), AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::delete_room(&st, &user, id).await?)
}

/// `moderator`: `Some(true)` moderator, `Some(false)` invited member,
/// `None` removes the user from the room's member list.
#[server(input = Json)]
pub async fn set_call_room_member(room_id: Uuid, user_id: Uuid, moderator: Option<bool>) -> Result<(), AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::set_member(&st, &user, room_id, user_id, moderator).await?)
}

#[server(input = Json)]
pub async fn join_call_room(id: Uuid) -> Result<JoinInfo, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::join_room(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn mute_call_participant(room_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::mute_participant(&st, &user, room_id, user_id).await?)
}

#[server(input = Json)]
pub async fn remove_call_participant(room_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::remove_participant(&st, &user, room_id, user_id).await?)
}

#[server(input = Json)]
pub async fn start_direct_call(user_id: Uuid, video: bool) -> Result<DirectCallDetail, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::start_call(&st, &user, user_id, video).await?)
}

#[server(input = Json)]
pub async fn get_direct_call(id: Uuid) -> Result<DirectCallDetail, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::direct_detail(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn answer_direct_call(id: Uuid, accept: bool) -> Result<DirectCallDetail, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::answer(&st, &user, id, accept).await?)
}

#[server(input = Json)]
pub async fn hangup_direct_call(id: Uuid) -> Result<DirectCallView, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::hangup(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn incoming_calls() -> Result<Vec<DirectCallView>, AppError> {
    let (st, user) = call_user().await?;
    Ok(backend::calls::incoming(&st.db, &user).await?)
}
