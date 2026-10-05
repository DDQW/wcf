use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::admin::AdminUserPage;
use shared::search::{SearchKind, SearchResults};
use shared::{AdminStats, SiteSettings};

use crate::error::AppError;

#[server(input = Json)]
pub async fn fetch_admin_stats() -> Result<AdminStats, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_mod().await?;
    Ok(backend::admin::stats(&st, &user).await?)
}

#[server(input = Json)]
pub async fn admin_users(search: Option<String>, filter: String, page: i64) -> Result<AdminUserPage, AppError> {
    let st = crate::ssr::state()?;
    crate::ssr::require_mod().await?;
    let per_page = 30;
    let (items, total) = backend::users::admin_list(&st.db, search, &filter, page, per_page).await?;
    Ok(AdminUserPage {
        items,
        total,
        per_page,
    })
}

#[server(input = Json)]
pub async fn get_site_settings() -> Result<SiteSettings, AppError> {
    let st = crate::ssr::state()?;
    crate::ssr::require_admin().await?;
    Ok(st.settings())
}

#[server(input = Json)]
pub async fn save_site_settings(settings: SiteSettings) -> Result<SiteSettings, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_admin().await?;
    Ok(backend::settings::save(&st, &user, settings).await?)
}

#[server(input = Json)]
pub async fn search(query: String, kind: Option<SearchKind>) -> Result<SearchResults, AppError> {
    let st = crate::ssr::state()?;
    let viewer = crate::ssr::viewer().await?;
    let ip = crate::ssr::client_ip().await;
    st.limiter
        .hit(&format!("search:{ip}"), 60, std::time::Duration::from_secs(60))
        .map_err(AppError::from)?;
    Ok(backend::search::search(&st, viewer.as_ref(), &query, kind).await?)
}

#[server(input = Json)]
pub async fn preview_markdown(markdown: String) -> Result<String, AppError> {
    let st = crate::ssr::state()?;
    crate::ssr::require_user().await?;
    if markdown.chars().count() > 60_000 {
        return Err(AppError::invalid("Der Text ist zu lang."));
    }
    Ok(backend::markdown::render_mentions(&st.db, &markdown)
        .await
        .map_err(backend::Error::from)?
        .html)
}
