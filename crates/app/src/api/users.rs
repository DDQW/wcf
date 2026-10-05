use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::user::{
    ActivityItem, MemberPage, MemberSort, NotifyPrefs, ProfileUpdate, ProfileView, SettingsView,
};
use shared::{HomeData, Role, Theme, UserSummary};
use uuid::Uuid;

use crate::error::AppError;

#[server(input = Json)]
pub async fn profile(username: String) -> Result<ProfileView, AppError> {
    let st = crate::ssr::state()?;
    let viewer = crate::ssr::viewer().await?;
    let mut p = backend::users::profile(&st.db, &username, viewer.as_ref()).await?;
    crate::api::extend_profile(&st, &mut p).await;
    Ok(p)
}

#[server(input = Json)]
pub async fn user_activity(user_id: Uuid) -> Result<Vec<ActivityItem>, AppError> {
    let st = crate::ssr::state()?;
    let viewer = crate::ssr::viewer().await?;
    let mut items = backend::forum::user_activity(&st.db, viewer.as_ref(), user_id, 30).await?;
    crate::api::extend_activity(&st, viewer.as_ref(), user_id, &mut items).await;
    items.sort_by_key(|a| std::cmp::Reverse(a.created_at));
    items.truncate(30);
    Ok(items)
}

#[server(input = Json)]
pub async fn members(
    search: Option<String>,
    role: Option<Role>,
    online_only: bool,
    sort: MemberSort,
    page: i64,
) -> Result<MemberPage, AppError> {
    let st = crate::ssr::state()?;
    let per_page = 24;
    let (items, total) = backend::users::members(
        &st.db,
        backend::users::MemberQuery {
            search,
            role,
            online_only: online_only.then(|| st.hub.online_users()),
            sort,
            page,
            per_page,
        },
    )
    .await?;
    Ok(MemberPage {
        items,
        total,
        per_page,
    })
}

#[server(input = Json)]
pub async fn online_users() -> Result<Vec<UserSummary>, AppError> {
    let st = crate::ssr::state()?;
    let ids = st.hub.online_users();
    let mut list: Vec<UserSummary> = backend::users::summaries(&st.db, &ids)
        .await?
        .into_values()
        .collect();
    list.sort_by_key(|a| a.display_name.to_lowercase());
    Ok(list)
}

#[server(input = Json)]
pub async fn mention_suggest(prefix: String) -> Result<Vec<UserSummary>, AppError> {
    let st = crate::ssr::state()?;
    crate::ssr::require_user().await?;
    Ok(backend::users::search_prefix(&st.db, &prefix, 8).await?)
}

#[server(input = Json)]
pub async fn my_settings() -> Result<SettingsView, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::users::settings(&st.db, user.id).await?)
}

#[server(input = Json)]
pub async fn update_profile(input: ProfileUpdate) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    backend::users::update_profile(&st.db, user.id, input).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn change_password(current: String, new_password: String) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    st.limiter
        .hit(&format!("pw:{}", user.id), 10, std::time::Duration::from_secs(3600))
        .map_err(AppError::from)?;
    backend::auth::change_password(&st.db, user.id, &current, &new_password).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn change_email(password: String, email: String) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    st.limiter
        .hit(&format!("pw:{}", user.id), 10, std::time::Duration::from_secs(3600))
        .map_err(AppError::from)?;
    backend::auth::change_email(&st.db, user.id, &password, &email).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn set_theme(theme: Theme) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    backend::users::update_theme(&st.db, user.id, theme).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn set_notify_prefs(prefs: NotifyPrefs) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    backend::users::update_notify_prefs(&st.db, user.id, &prefs).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn fetch_home_data() -> Result<HomeData, AppError> {
    use shared::forum::{TopicFilter, TopicSort};
    let st = crate::ssr::state()?;
    let viewer = crate::ssr::viewer().await?;
    let latest_topics = if st.settings().modules.forum {
        backend::forum::list_topics(&st.db, viewer.as_ref(), TopicFilter::All, TopicSort::Activity, None, Some(8))
            .await?
            .items
    } else {
        Vec::new()
    };
    let ids = st.hub.online_users();
    let mut online: Vec<UserSummary> = backend::users::summaries(&st.db, &ids)
        .await?
        .into_values()
        .collect();
    online.sort_by_key(|a| a.display_name.to_lowercase());
    let newest_members = backend::users::newest(&st.db, 6).await?;
    let mut data = HomeData {
        latest_topics,
        online,
        newest_members,
        ..Default::default()
    };
    crate::api::fill_home(&st, viewer.as_ref(), &mut data).await?;
    Ok(data)
}
