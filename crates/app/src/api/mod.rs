//! Leptos server functions — the API between the WASM client and the
//! backend crate. Each function re-checks authentication server-side.

pub mod admin;
pub mod auth;
pub mod blog;
pub mod chat;
pub mod cms;
pub mod files;
pub mod forum;
pub mod gallery;
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
    pub async fn nav_links(st: &AppState) -> Vec<NavLink> {
        if !st.settings().modules.cms {
            return Vec::new();
        }
        backend::cms::menu(&st.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| NavLink { label: m.label, url: m.url })
            .collect()
    }

    /// Lets later modules add profile statistics.
    pub async fn extend_profile(st: &AppState, p: &mut ProfileView) {
        if st.settings().modules.blogs {
            p.has_blog = matches!(backend::blogs::blog_of(&st.db, None, p.user.id).await, Ok(Some(_)));
        }
    }

    /// Lets later modules add items to a user's activity stream.
    pub async fn extend_activity(
        st: &AppState,
        viewer: Option<&AuthUser>,
        user_id: Uuid,
        items: &mut Vec<ActivityItem>,
    ) {
        if st.settings().modules.gallery {
            if let Ok(page) = backend::gallery::latest_images(&st.db, viewer, Some(user_id), None, Some(5)).await {
                items.extend(page.items.into_iter().map(|i| ActivityItem {
                    icon: "image".into(),
                    label: "Bild hochgeladen".into(),
                    title: i.display_title(),
                    excerpt: i.description.clone(),
                    link: i.url(),
                    created_at: i.created_at,
                }));
            }
        }
    }

    /// Lets later modules add sections to the start page.
    pub async fn fill_home(st: &AppState, _viewer: Option<&AuthUser>, data: &mut HomeData) -> Result<(), AppError> {
        let counts: (i64, i64, i64) = sqlx_counts(st).await?;
        data.stats.members = counts.0;
        data.stats.topics = counts.1;
        data.stats.posts = counts.2;
        if st.settings().modules.blogs {
            data.extra.latest_posts = backend::blogs::list_posts(
                &st.db,
                _viewer,
                shared::blog::BlogFilter::All,
                None,
                Some(3),
            )
            .await?
            .items;
        }
        if st.settings().modules.gallery {
            data.extra.latest_images = backend::gallery::latest_images(&st.db, _viewer, None, None, Some(8))
                .await?
                .items;
        }
        Ok(())
    }

    async fn sqlx_counts(st: &AppState) -> Result<(i64, i64, i64), AppError> {
        let stats = backend::admin::community_counts(&st.db).await?;
        Ok(stats)
    }
}

#[cfg(feature = "ssr")]
pub(crate) use hooks::*;
