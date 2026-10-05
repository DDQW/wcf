use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::forum::{
    BlockPage, CategoryInput, CategoryView, ForumOverview, NewTopic, PostRevision, PostView,
    ReplyPage, SubscriptionLevel, TopicDetail, TopicFilter, TopicPage, TopicSort, TopicUpdate,
};
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
fn forum_state() -> Result<backend::AppState, AppError> {
    crate::ssr::require_module(|m| m.forum)
}

#[server(input = Json)]
pub async fn fetch_forum_overview() -> Result<ForumOverview, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::overview(&st.db, viewer.as_ref()).await?)
}

#[server(input = Json)]
pub async fn forum_categories() -> Result<Vec<CategoryView>, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::categories(&st.db, viewer.as_ref()).await?)
}

#[server(input = Json)]
pub async fn forum_category(id: Uuid) -> Result<CategoryView, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::category(&st.db, viewer.as_ref(), id).await?)
}

#[server(input = Json)]
pub async fn list_topics(
    filter: TopicFilter,
    sort: TopicSort,
    cursor: Option<String>,
) -> Result<TopicPage, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::list_topics(&st.db, viewer.as_ref(), filter, sort, cursor, None).await?)
}

#[server(input = Json)]
pub async fn get_topic(topic_id: Uuid) -> Result<TopicDetail, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::forum::get_topic(&st, viewer.as_ref(), topic_id).await;
    if matches!(res, Err(backend::Error::NotFound(_))) {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn list_blocks(
    topic_id: Uuid,
    after: Option<String>,
    before: Option<String>,
    from_post: Option<Uuid>,
) -> Result<BlockPage, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::list_blocks(
        &st.db,
        viewer.as_ref(),
        topic_id,
        backend::forum::BlockQuery {
            after,
            before,
            from_post,
        },
    )
    .await?)
}

#[server(input = Json)]
pub async fn list_replies(root_id: Uuid, after: Option<String>) -> Result<ReplyPage, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::list_replies(&st.db, viewer.as_ref(), root_id, after).await?)
}

#[server(input = Json)]
pub async fn create_topic(input: NewTopic) -> Result<Uuid, AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::create_topic(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn create_post(topic_id: Uuid, parent_id: Option<Uuid>, body_md: String) -> Result<PostView, AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::create_post(&st, &user, topic_id, parent_id, &body_md).await?)
}

#[server(input = Json)]
pub async fn edit_post(post_id: Uuid, body_md: String) -> Result<PostView, AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::edit_post(&st, &user, post_id, &body_md).await?)
}

#[server(input = Json)]
pub async fn delete_post(post_id: Uuid) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::delete_post(&st, &user, post_id).await?)
}

#[server(input = Json)]
pub async fn restore_post(post_id: Uuid) -> Result<PostView, AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::restore_post(&st, &user, post_id).await?)
}

#[server(input = Json)]
pub async fn post_source(post_id: Uuid) -> Result<String, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::post_source(&st.db, viewer.as_ref(), post_id).await?)
}

#[server(input = Json)]
pub async fn post_revisions(post_id: Uuid) -> Result<Vec<PostRevision>, AppError> {
    let st = forum_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::forum::post_revisions(&st.db, viewer.as_ref(), post_id).await?)
}

#[server(input = Json)]
pub async fn update_topic(topic_id: Uuid, input: TopicUpdate) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::update_topic(&st, &user, topic_id, input).await?)
}

#[server(input = Json)]
pub async fn set_topic_pinned(topic_id: Uuid, pinned: bool) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::set_pinned(&st, &user, topic_id, pinned).await?)
}

#[server(input = Json)]
pub async fn set_topic_locked(topic_id: Uuid, locked: bool) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::set_locked(&st, &user, topic_id, locked).await?)
}

#[server(input = Json)]
pub async fn move_topic(topic_id: Uuid, category_id: Uuid) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::move_topic(&st, &user, topic_id, category_id).await?)
}

#[server(input = Json)]
pub async fn delete_topic(topic_id: Uuid) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::delete_topic(&st, &user, topic_id).await?)
}

#[server(input = Json)]
pub async fn restore_topic(topic_id: Uuid) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::restore_topic(&st, &user, topic_id).await?)
}

#[server(input = Json)]
pub async fn set_topic_subscription(topic_id: Uuid, level: SubscriptionLevel) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::forum::set_subscription(&st.db, &user, topic_id, level).await?)
}

#[server(input = Json)]
pub async fn save_category(id: Option<Uuid>, input: CategoryInput) -> Result<Uuid, AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_admin().await?;
    match id {
        Some(id) => {
            backend::forum::update_category(&st, &user, id, input).await?;
            Ok(id)
        }
        None => Ok(backend::forum::create_category(&st, &user, input).await?),
    }
}

#[server(input = Json)]
pub async fn delete_category(id: Uuid, move_to: Option<Uuid>) -> Result<(), AppError> {
    let st = forum_state()?;
    let user = crate::ssr::require_admin().await?;
    Ok(backend::forum::delete_category(&st, &user, id, move_to).await?)
}
