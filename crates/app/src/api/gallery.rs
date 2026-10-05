use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::gallery::{AlbumDetail, AlbumFilter, AlbumInput, AlbumPage, AlbumView, ImageDetail, ImagePage};
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
fn gallery_state() -> Result<backend::AppState, AppError> {
    crate::ssr::require_module(|m| m.gallery)
}

#[cfg(feature = "ssr")]
fn not_found_status<T>(res: &Result<T, backend::Error>) {
    if matches!(res, Err(backend::Error::NotFound(_))) {
        crate::ssr::set_status(404);
    }
}

#[server(input = Json)]
pub async fn list_albums(filter: AlbumFilter, page: i64) -> Result<AlbumPage, AppError> {
    let st = gallery_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::gallery::list_albums(&st.db, viewer.as_ref(), filter, page).await?)
}

#[server(input = Json)]
pub async fn get_album(id: Uuid) -> Result<AlbumDetail, AppError> {
    let st = gallery_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::gallery::album_detail(&st.db, viewer.as_ref(), id).await;
    not_found_status(&res);
    Ok(res?)
}

#[server(input = Json)]
pub async fn upload_albums() -> Result<Vec<AlbumView>, AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::upload_targets(&st.db, &user).await?)
}

#[server(input = Json)]
pub async fn create_album(input: AlbumInput) -> Result<AlbumView, AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::create_album(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn update_album(id: Uuid, input: AlbumInput) -> Result<AlbumView, AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::update_album(&st, &user, id, input).await?)
}

#[server(input = Json)]
pub async fn delete_album(id: Uuid) -> Result<(), AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::delete_album(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn set_album_cover(album_id: Uuid, image_id: Option<Uuid>) -> Result<(), AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::set_cover(&st.db, &user, album_id, image_id).await?)
}

#[server(input = Json)]
pub async fn reorder_album(album_id: Uuid, ids: Vec<Uuid>) -> Result<(), AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::reorder(&st.db, &user, album_id, &ids).await?)
}

#[server(input = Json)]
pub async fn get_image(id: Uuid) -> Result<ImageDetail, AppError> {
    let st = gallery_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::gallery::image_detail(&st.db, viewer.as_ref(), id).await;
    not_found_status(&res);
    Ok(res?)
}

#[server(input = Json)]
pub async fn update_image(id: Uuid, title: String, description: String, album_id: Option<Uuid>) -> Result<(), AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::update_image(&st, &user, id, &title, &description, album_id).await?)
}

#[server(input = Json)]
pub async fn delete_image(id: Uuid) -> Result<(), AppError> {
    let st = gallery_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::gallery::delete_image(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn latest_images(user: Option<Uuid>, cursor: Option<String>) -> Result<ImagePage, AppError> {
    let st = gallery_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::gallery::latest_images(&st.db, viewer.as_ref(), user, cursor, None).await?)
}
