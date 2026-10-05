use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::cms::{
    CmsTypeInput, CmsTypeView, EntryEdit, EntryPage, EntryQuery, EntryView, MenuItem, RelationRef,
    RevisionView,
};
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
fn cms_state() -> Result<backend::AppState, AppError> {
    crate::ssr::require_module(|m| m.cms)
}

#[server(input = Json)]
pub async fn cms_types() -> Result<Vec<CmsTypeView>, AppError> {
    let st = cms_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::cms::list_types(&st.db, viewer.as_ref(), false).await?)
}

#[server(input = Json)]
pub async fn admin_cms_types() -> Result<Vec<CmsTypeView>, AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_admin().await?;
    Ok(backend::cms::list_types(&st.db, Some(&user), true).await?)
}

#[server(input = Json)]
pub async fn cms_type(slug: String) -> Result<CmsTypeView, AppError> {
    let st = cms_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::cms::get_type(&st.db, viewer.as_ref(), &slug).await;
    if res.is_err() {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn save_cms_type(id: Option<Uuid>, input: CmsTypeInput) -> Result<Uuid, AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_admin().await?;
    Ok(backend::cms::save_type(&st, &user, id, input).await?)
}

#[server(input = Json)]
pub async fn delete_cms_type(id: Uuid) -> Result<(), AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_admin().await?;
    Ok(backend::cms::delete_type(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn cms_entries(type_slug: String, query: EntryQuery) -> Result<EntryPage, AppError> {
    let st = cms_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::cms::list_entries(&st, viewer.as_ref(), &type_slug, query).await?)
}

#[server(input = Json)]
pub async fn cms_entry(type_slug: String, slug: String) -> Result<EntryView, AppError> {
    let st = cms_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::cms::get_entry(&st, viewer.as_ref(), &type_slug, &slug).await;
    if res.is_err() {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn cms_page(slug: String) -> Result<EntryView, AppError> {
    let st = cms_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::cms::page(&st, viewer.as_ref(), &slug).await;
    if res.is_err() {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn cms_entry_for_edit(id: Uuid) -> Result<EntryEdit, AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::cms::entry_for_edit(&st.db, &user, id).await?)
}

#[server(input = Json)]
pub async fn save_cms_entry(input: EntryEdit) -> Result<String, AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::cms::save_entry(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn delete_cms_entry(id: Uuid) -> Result<String, AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::cms::delete_entry(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn cms_revisions(id: Uuid) -> Result<Vec<RevisionView>, AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::cms::revisions(&st.db, &user, id).await?)
}

#[server(input = Json)]
pub async fn restore_cms_revision(id: Uuid) -> Result<String, AppError> {
    let st = cms_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::cms::restore_revision(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn cms_relation_options(type_slug: String, query: String) -> Result<Vec<RelationRef>, AppError> {
    let st = cms_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::cms::relation_options(&st, viewer.as_ref(), &type_slug, &query).await?)
}

#[server(input = Json)]
pub async fn cms_relation_titles(ids: Vec<Uuid>) -> Result<Vec<RelationRef>, AppError> {
    let st = cms_state()?;
    crate::ssr::require_user().await?;
    Ok(backend::cms::relation_titles(&st.db, &ids).await?)
}

#[server(input = Json)]
pub async fn cms_menu() -> Result<Vec<MenuItem>, AppError> {
    let st = crate::ssr::state()?;
    Ok(backend::cms::menu(&st.db).await?)
}

#[server(input = Json)]
pub async fn save_cms_menu(items: Vec<MenuItem>) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_admin().await?;
    Ok(backend::cms::save_menu(&st, &user, items).await?)
}
