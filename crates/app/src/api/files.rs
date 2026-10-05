use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::files::{
    DriveListing, DriveSort, FilePurpose, FileView, FolderView, ShareInfo, ShareView, Visibility,
};
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
async fn files_user() -> Result<(backend::AppState, shared::AuthUser), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok((st, user))
}

#[server(input = Json)]
pub async fn list_drive(folder_id: Option<Uuid>, sort: DriveSort, search: Option<String>) -> Result<DriveListing, AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::list_drive(&st, &user, folder_id, sort, search).await?)
}

#[server(input = Json)]
pub async fn list_trash() -> Result<DriveListing, AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::list_trash(&st, &user).await?)
}

#[server(input = Json)]
pub async fn folder_tree() -> Result<Vec<FolderView>, AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::folder_tree(&st.db, &user).await?)
}

#[server(input = Json)]
pub async fn create_folder(parent: Option<Uuid>, name: String) -> Result<FolderView, AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::create_folder(&st.db, &user, parent, &name).await?)
}

#[server(input = Json)]
pub async fn rename_folder(id: Uuid, name: String) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::rename_folder(&st.db, &user, id, &name).await?)
}

#[server(input = Json)]
pub async fn move_folder(id: Uuid, parent: Option<Uuid>) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::move_folder(&st.db, &user, id, parent).await?)
}

#[server(input = Json)]
pub async fn trash_folder(id: Uuid) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::trash_folder(&st.db, &user, id).await?)
}

#[server(input = Json)]
pub async fn restore_folder(id: Uuid) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::restore_folder(&st.db, &user, id).await?)
}

#[server(input = Json)]
pub async fn purge_folder(id: Uuid) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::purge_folder(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn rename_file(id: Uuid, name: String, description: String) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::rename_file(&st, &user, id, &name, &description).await?)
}

#[server(input = Json)]
pub async fn move_files(ids: Vec<Uuid>, folder: Option<Uuid>) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::move_files(&st, &user, &ids, folder).await?)
}

#[server(input = Json)]
pub async fn set_files_visibility(ids: Vec<Uuid>, visibility: Visibility) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::set_visibility(&st, &user, &ids, visibility).await?)
}

#[server(input = Json)]
pub async fn trash_files(ids: Vec<Uuid>) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::trash_files(&st, &user, &ids).await?)
}

#[server(input = Json)]
pub async fn restore_files(ids: Vec<Uuid>) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::restore_files(&st, &user, &ids).await?)
}

#[server(input = Json)]
pub async fn purge_files(ids: Vec<Uuid>) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::purge_files(&st, &user, &ids).await?)
}

#[server(input = Json)]
pub async fn empty_trash() -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::empty_trash(&st, &user).await?)
}

#[server(input = Json)]
pub async fn file_info(id: Uuid) -> Result<FileView, AppError> {
    let st = crate::ssr::state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::files::file_info(&st, viewer.as_ref(), id).await?)
}

#[server(input = Json)]
pub async fn my_uploads(purpose: FilePurpose) -> Result<Vec<FileView>, AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::list_by_purpose(&st.db, &user, purpose, 200).await?)
}

#[server(input = Json)]
pub async fn create_share(
    file_id: Option<Uuid>,
    folder_id: Option<Uuid>,
    expires_days: Option<i64>,
    password: Option<String>,
    max_downloads: Option<i32>,
) -> Result<ShareView, AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::create_share(&st, &user, file_id, folder_id, expires_days, password, max_downloads).await?)
}

#[server(input = Json)]
pub async fn list_shares(file_id: Option<Uuid>, folder_id: Option<Uuid>) -> Result<Vec<ShareView>, AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::list_shares(&st.db, &user, file_id, folder_id).await?)
}

#[server(input = Json)]
pub async fn revoke_share(id: Uuid) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    Ok(backend::files::revoke_share(&st.db, &user, id).await?)
}

#[cfg(feature = "ssr")]
async fn unlocked(token: &str) -> Result<bool, AppError> {
    let session = crate::ssr::session().await?;
    let list: Vec<String> = session
        .get(backend::files::SESSION_SHARES_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(list.iter().any(|t| t == token))
}

#[server(input = Json)]
pub async fn get_share_info(token: String) -> Result<ShareInfo, AppError> {
    let st = crate::ssr::state()?;
    let ok = unlocked(&token).await?;
    Ok(backend::files::share_info(&st, &token, ok).await?)
}

#[server(input = Json)]
pub async fn unlock_share(token: String, password: String) -> Result<ShareInfo, AppError> {
    let st = crate::ssr::state()?;
    let ip = crate::ssr::client_ip().await;
    st.limiter
        .hit(&format!("share-pw:{ip}"), 10, std::time::Duration::from_secs(600))
        .map_err(AppError::from)?;
    if !backend::files::verify_share_password(&st.db, &token, &password).await? {
        return Err(AppError::invalid("Falsches Passwort."));
    }
    let session = crate::ssr::session().await?;
    let mut list: Vec<String> = session
        .get(backend::files::SESSION_SHARES_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if !list.contains(&token) {
        list.push(token.clone());
        list.truncate(50);
    }
    session
        .insert(backend::files::SESSION_SHARES_KEY, list)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(backend::files::share_info(&st, &token, true).await?)
}

/// Sets (or clears) the avatar. The file must be an own image uploaded
/// with purpose `avatar`.
#[server(input = Json)]
pub async fn set_avatar(file_id: Option<Uuid>) -> Result<(), AppError> {
    let (st, user) = files_user().await?;
    if let Some(id) = file_id {
        let rec = backend::files::get_record(&st.db, id)
            .await?
            .ok_or_else(|| AppError::NotFound("Datei nicht gefunden.".into()))?;
        if rec.owner_id != user.id || rec.view.purpose != FilePurpose::Avatar || !rec.view.mime_type.starts_with("image/") {
            return Err(AppError::invalid("Bitte lade ein Bild hoch."));
        }
    }
    backend::users::set_avatar(&st.db, user.id, file_id).await?;
    Ok(())
}
