use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::blog::{
    BlogFilter, BlogPostDetail, BlogPostEdit, BlogPostItem, BlogPostPage, BlogView, CommentView,
    SavedPost,
};
use shared::forum::TagCount;
use shared::SubjectType;
use uuid::Uuid;

use crate::error::AppError;

#[cfg(feature = "ssr")]
fn blog_state() -> Result<backend::AppState, AppError> {
    crate::ssr::require_module(|m| m.blogs)
}

#[server(input = Json)]
pub async fn list_blog_posts(filter: BlogFilter, cursor: Option<String>) -> Result<BlogPostPage, AppError> {
    let st = blog_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::blogs::list_posts(&st.db, viewer.as_ref(), filter, cursor, None).await?)
}

#[server(input = Json)]
pub async fn all_blogs() -> Result<Vec<BlogView>, AppError> {
    let st = blog_state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::blogs::all_blogs(&st.db, viewer.as_ref()).await?)
}

#[server(input = Json)]
pub async fn blog_tags() -> Result<Vec<TagCount>, AppError> {
    let st = blog_state()?;
    Ok(backend::blogs::popular_tags(&st.db, 30).await?)
}

#[server(input = Json)]
pub async fn get_blog(username: String) -> Result<BlogView, AppError> {
    let st = blog_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::blogs::blog_by_username(&st.db, viewer.as_ref(), &username).await;
    if res.is_err() {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn get_blog_post(username: String, slug: String) -> Result<BlogPostDetail, AppError> {
    let st = blog_state()?;
    let viewer = crate::ssr::viewer().await?;
    let res = backend::blogs::get_post(&st, viewer.as_ref(), &username, &slug).await;
    if matches!(res, Err(backend::Error::NotFound(_))) {
        crate::ssr::set_status(404);
    }
    Ok(res?)
}

#[server(input = Json)]
pub async fn my_blog() -> Result<Option<BlogView>, AppError> {
    let st = blog_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::blogs::blog_of(&st.db, Some(&user), user.id).await?)
}

#[server(input = Json)]
pub async fn save_my_blog(title: String, description: String) -> Result<BlogView, AppError> {
    let st = blog_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::blogs::save_blog(&st.db, &user, &title, &description).await?)
}

#[server(input = Json)]
pub async fn my_blog_posts() -> Result<Vec<BlogPostItem>, AppError> {
    let st = blog_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::blogs::my_posts(&st.db, &user).await?)
}

#[server(input = Json)]
pub async fn blog_post_for_edit(id: Uuid) -> Result<BlogPostEdit, AppError> {
    let st = blog_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::blogs::post_for_edit(&st.db, &user, id).await?)
}

#[server(input = Json)]
pub async fn save_blog_post(input: BlogPostEdit) -> Result<SavedPost, AppError> {
    let st = blog_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::blogs::save_post(&st, &user, input).await?)
}

#[server(input = Json)]
pub async fn delete_blog_post(id: Uuid) -> Result<(), AppError> {
    let st = blog_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::blogs::delete_post(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn subscribe_blog(blog_id: Uuid, subscribed: bool) -> Result<(), AppError> {
    let st = blog_state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::blogs::set_subscription(&st, &user, blog_id, subscribed).await?)
}

// --- comments (generic) -----------------------------------------------------

#[server(input = Json)]
pub async fn list_comments(subject_type: SubjectType, subject_id: Uuid) -> Result<Vec<CommentView>, AppError> {
    let st = crate::ssr::state()?;
    let viewer = crate::ssr::viewer().await?;
    Ok(backend::comments::list(&st, viewer.as_ref(), subject_type, subject_id).await?)
}

#[server(input = Json)]
pub async fn add_comment(
    subject_type: SubjectType,
    subject_id: Uuid,
    parent_id: Option<Uuid>,
    body: String,
) -> Result<CommentView, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::comments::create(&st, &user, subject_type, subject_id, parent_id, &body).await?)
}

#[server(input = Json)]
pub async fn comment_source(id: Uuid) -> Result<String, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::comments::source(&st, &user, id).await?)
}

#[server(input = Json)]
pub async fn edit_comment(id: Uuid, body: String) -> Result<String, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::comments::edit(&st, &user, id, &body).await?)
}

#[server(input = Json)]
pub async fn delete_comment(id: Uuid) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::comments::delete(&st, &user, id).await?)
}
