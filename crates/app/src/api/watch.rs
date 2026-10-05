use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::watch::{
    Emote, EventInfo, EventInput, EventSaved, ImportFile, ImportMode, LibraryOverview, MediaInfo, ModerationKind,
    PackageSelection, PlaylistSource, ProposalInfo, ProposalInput, ProposalStatus, RestrictedUser, RoomRole,
    RoomRoleRow, WatchRoomInput, WatchRoomView,
};
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
async fn watch_user() -> Result<(backend::AppState, shared::AuthUser), AppError> {
    let st = crate::ssr::require_module(|m| m.watch)?;
    let user = crate::ssr::require_user().await?;
    Ok((st, user))
}

// ---- rooms ----

#[server(input = Json)]
pub async fn fetch_watch_rooms() -> Result<Vec<WatchRoomView>, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::list_rooms(&st, &user).await?)
}

#[server(input = Json)]
pub async fn get_watch_room(slug: String) -> Result<WatchRoomView, AppError> {
    let (st, user) = watch_user().await?;
    let res = backend::watch::room_view_by_slug(&st, &user, &slug).await;
    if matches!(res, Err(backend::Error::NotFound(_))) {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn create_watch_room(input: WatchRoomInput) -> Result<WatchRoomView, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::create_room(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn update_watch_room(id: Uuid, input: WatchRoomInput) -> Result<WatchRoomView, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::update_room(&st, &user, id, input).await?)
}

#[server(input = Json)]
pub async fn delete_watch_room(id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::delete_room(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn set_watch_slow_mode(id: Uuid, ms: i64) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::set_slow_mode(&st, &user, id, ms).await?)
}

#[server(input = Json)]
pub async fn watch_room_roles(id: Uuid) -> Result<Vec<RoomRoleRow>, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::roles(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn set_watch_role(room_id: Uuid, user_id: Uuid, role: Option<RoomRole>) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::set_role(&st, &user, room_id, user_id, role).await?)
}

// ---- chat + moderation ----

#[server(input = Json)]
pub async fn delete_watch_chat(room_id: Uuid, message_id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::delete_chat(&st, &user, room_id, message_id).await?)
}

#[server(input = Json)]
pub async fn moderate_watch_user(
    room_id: Uuid,
    user_id: Uuid,
    kind: ModerationKind,
    minutes: Option<i64>,
    reason: String,
) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::moderate(&st, &user, room_id, user_id, kind, minutes, &reason).await?)
}

#[server(input = Json)]
pub async fn watch_restricted(room_id: Uuid) -> Result<Vec<RestrictedUser>, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::restricted(&st, &user, room_id).await?)
}

// ---- requests ----

#[server(input = Json)]
pub async fn watch_proposals(room_id: Uuid, all: bool) -> Result<Vec<ProposalInfo>, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::proposals(&st, &user, room_id, all).await?)
}

#[server(input = Json)]
pub async fn create_watch_proposal(room_id: Uuid, input: ProposalInput) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::create_proposal(&st, &user, room_id, input).await?)
}

#[server(input = Json)]
pub async fn vote_watch_proposal(id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::toggle_vote(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn delete_watch_proposal(id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::delete_proposal(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn set_watch_proposal_status(id: Uuid, status: ProposalStatus) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::set_proposal_status(&st, &user, id, status).await?)
}

// ---- emotes ----

#[server(input = Json)]
pub async fn watch_emotes() -> Result<Vec<Emote>, AppError> {
    let (st, _) = watch_user().await?;
    Ok(backend::watch::emotes(&st.db).await?)
}

#[server(input = Json)]
pub async fn create_watch_emote(code: String, file_id: Uuid) -> Result<Emote, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::create_emote(&st, &user, &code, file_id).await?)
}

#[server(input = Json)]
pub async fn delete_watch_emote(id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::delete_emote(&st, &user, id).await?)
}

// ---- shows ----

#[server(input = Json)]
pub async fn watch_events(room_id: Uuid, include_past: bool) -> Result<Vec<EventInfo>, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::events(&st, &user, room_id, include_past).await?)
}

#[server(input = Json)]
pub async fn create_watch_event(room_id: Uuid, input: EventInput) -> Result<EventSaved, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::create_event(&st, &user, room_id, input).await?)
}

#[server(input = Json)]
pub async fn update_watch_event(id: Uuid, input: EventInput) -> Result<EventSaved, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::update_event(&st, &user, id, input).await?)
}

#[server(input = Json)]
pub async fn cancel_watch_event(id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::cancel_event(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn delete_watch_event(id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::delete_event(&st, &user, id).await?)
}

// ---- playlist ----

#[server(input = Json)]
pub async fn add_watch_item(room_id: Uuid, source: PlaylistSource, next: bool) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_live::add_to_playlist(&st, &user, room_id, source, next).await?)
}

#[server(input = Json)]
pub async fn remove_watch_item(room_id: Uuid, item_id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_live::remove_from_playlist(&st, &user, room_id, item_id).await?)
}

#[server(input = Json)]
pub async fn move_watch_item(room_id: Uuid, item_id: Uuid, before: Option<Uuid>) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_live::move_in_playlist(&st, &user, room_id, item_id, before).await?)
}

#[server(input = Json)]
pub async fn shuffle_watch_playlist(room_id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_live::shuffle_playlist(&st, &user, room_id).await?)
}

#[server(input = Json)]
pub async fn clear_watch_playlist(room_id: Uuid) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_live::clear_playlist(&st, &user, room_id).await?)
}

// ---- media library ----

#[server(input = Json)]
pub async fn watch_library() -> Result<LibraryOverview, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_media::library(&st, &user).await?)
}

#[server(input = Json)]
pub async fn watch_import_files() -> Result<Vec<ImportFile>, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_media::import_files(&st, &user).await?)
}

#[server(input = Json)]
pub async fn import_watch_media(title: String, path: String, mode: ImportMode) -> Result<MediaInfo, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_media::import(&st, &user, &title, &path, mode).await?)
}

#[server(input = Json)]
pub async fn package_watch_media(id: Uuid, selection: PackageSelection) -> Result<(), AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_media::package(&st, &user, id, selection).await?)
}

#[server(input = Json)]
pub async fn rename_watch_media(id: Uuid, title: String) -> Result<MediaInfo, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_media::rename(&st, &user, id, &title).await?)
}

#[server(input = Json)]
pub async fn delete_watch_media(id: Uuid) -> Result<bool, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch_media::delete(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn my_watch_videos() -> Result<Vec<shared::files::FileView>, AppError> {
    let (st, user) = watch_user().await?;
    Ok(backend::watch::my_videos(&st.db, &user).await?)
}
