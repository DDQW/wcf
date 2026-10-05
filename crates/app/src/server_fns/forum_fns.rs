use leptos::prelude::*;
#[cfg(feature = "ssr")]
use shared::{Cursor, DEFAULT_PAGE_SIZE};
use shared::{CategorySummary, Page, PostView, TopicSummary};
use uuid::Uuid;

#[server]
pub async fn list_categories() -> Result<Vec<CategorySummary>, ServerFnError> {
    use crate::ssr::db::DbPool;
    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;
    crate::ssr::db::forum::list_categories(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server]
pub async fn create_category(
    name: String,
    description: String,
) -> Result<CategorySummary, ServerFnError> {
    use crate::ssr::db::DbPool;
    let user = crate::ssr::auth::require_user().await?;
    if !user.is_admin() {
        return Err(ServerFnError::new("only admins can create categories"));
    }
    if name.trim().is_empty() {
        return Err(ServerFnError::new("name is required"));
    }
    let description = description.trim();
    let description = (!description.is_empty()).then_some(description);
    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;
    crate::ssr::db::forum::create_category(&pool, &name, description)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server]
pub async fn get_topic(topic_id: Uuid) -> Result<TopicSummary, ServerFnError> {
    use crate::ssr::db::DbPool;
    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;
    crate::ssr::db::forum::get_topic(&pool, topic_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("topic not found"))
}

#[server]
pub async fn list_topics(
    category_id: Uuid,
    cursor: Option<String>,
) -> Result<Page<TopicSummary>, ServerFnError> {
    use crate::ssr::db::DbPool;
    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;
    let cursor_parsed = cursor
        .map(|c| Cursor::decode(&c))
        .transpose()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let items = crate::ssr::db::forum::list_topics(
        &pool,
        category_id,
        cursor_parsed.map(|c| (c.ts, c.id)),
        None,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    let next_cursor = if items.len() as i64 == DEFAULT_PAGE_SIZE {
        items.last().map(|t| Cursor::new(t.last_post_at, t.id).encode())
    } else {
        None
    };

    Ok(Page::new(items, next_cursor))
}

#[server]
pub async fn create_topic(
    category_id: Uuid,
    title: String,
    body_md: String,
) -> Result<Uuid, ServerFnError> {
    use crate::ssr::db::DbPool;
    let user = crate::ssr::auth::require_user().await?;
    if title.trim().is_empty() {
        return Err(ServerFnError::new("title is required"));
    }
    if body_md.trim().is_empty() {
        return Err(ServerFnError::new("body is required"));
    }
    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;
    crate::ssr::db::forum::create_topic(&pool, category_id, user.id, &title, &body_md)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server]
pub async fn list_posts(
    topic_id: Uuid,
    cursor: Option<String>,
) -> Result<Page<PostView>, ServerFnError> {
    use crate::ssr::db::DbPool;
    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;
    let cursor_parsed = cursor
        .map(|c| Cursor::decode(&c))
        .transpose()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let items = crate::ssr::db::forum::list_posts(
        &pool,
        topic_id,
        cursor_parsed.map(|c| (c.ts, c.id)),
        None,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    let next_cursor = if items.len() as i64 == DEFAULT_PAGE_SIZE {
        items.last().map(|p| Cursor::new(p.created_at, p.id).encode())
    } else {
        None
    };

    Ok(Page::new(items, next_cursor))
}

#[server]
pub async fn create_post(
    topic_id: Uuid,
    // Empty string means a fresh top-level reply (no parent) — kept as a
    // plain String because HTML forms submit "" for an unfilled field,
    // which does not round-trip cleanly through Option<Uuid>.
    parent_post_id: String,
    body_md: String,
) -> Result<Uuid, ServerFnError> {
    use crate::ssr::db::DbPool;
    let user = crate::ssr::auth::require_user().await?;
    if body_md.trim().is_empty() {
        return Err(ServerFnError::new("body is required"));
    }
    let parent_post_id = if parent_post_id.trim().is_empty() {
        None
    } else {
        Some(
            Uuid::parse_str(parent_post_id.trim())
                .map_err(|_| ServerFnError::new("invalid parent post id"))?,
        )
    };
    let pool =
        use_context::<DbPool>().ok_or_else(|| ServerFnError::new("db pool not available"))?;
    crate::ssr::db::forum::create_post(&pool, topic_id, user.id, parent_post_id, &body_md)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
