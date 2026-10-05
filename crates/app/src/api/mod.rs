//! Leptos server functions — the API between the WASM client and the
//! backend crate. Each function re-checks authentication server-side.

pub mod admin;
pub mod auth;
pub mod files;
pub mod forum;
pub mod moderation;
pub mod notifications;
pub mod users;

#[cfg(feature = "ssr")]
mod hooks {
    use backend::AppState;
    use shared::user::{ActivityItem, ProfileView};
    use shared::{AuthUser, HomeData, NavLink};
    use uuid::Uuid;

    use crate::error::AppError;

    /// Extra navigation links (CMS menu) for the shell.
    pub async fn nav_links(_st: &AppState) -> Vec<NavLink> {
        Vec::new()
    }

    /// Lets later modules add profile statistics.
    pub async fn extend_profile(_st: &AppState, _p: &mut ProfileView) {}

    /// Lets later modules add items to a user's activity stream.
    pub async fn extend_activity(
        _st: &AppState,
        _viewer: Option<&AuthUser>,
        _user_id: Uuid,
        _items: &mut Vec<ActivityItem>,
    ) {
    }

    /// Lets later modules add sections to the start page.
    pub async fn fill_home(st: &AppState, _viewer: Option<&AuthUser>, data: &mut HomeData) -> Result<(), AppError> {
        let counts: (i64, i64, i64) = sqlx_counts(st).await?;
        data.stats.members = counts.0;
        data.stats.topics = counts.1;
        data.stats.posts = counts.2;
        Ok(())
    }

    async fn sqlx_counts(st: &AppState) -> Result<(i64, i64, i64), AppError> {
        let stats = backend::admin::community_counts(&st.db).await?;
        Ok(stats)
    }
}

#[cfg(feature = "ssr")]
pub(crate) use hooks::*;
