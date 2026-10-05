use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorySummary {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicSummary {
    pub id: Uuid,
    pub category_id: Uuid,
    pub title: String,
    pub slug: String,
    pub author_display_name: String,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub reply_count: i32,
    pub last_post_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// A single post in a topic. Posts form threaded "blocks": every reply
/// (direct or nested) shares the `thread_root_id` of the top-level post
/// that started the conversation, so the client can group + collapse them
/// without a recursive tree walk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostView {
    pub id: Uuid,
    pub topic_id: Uuid,
    pub author_id: Uuid,
    pub author_display_name: String,
    pub parent_post_id: Option<Uuid>,
    pub thread_root_id: Uuid,
    pub depth: i16,
    pub body_html: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTopicRequest {
    pub category_id: Uuid,
    pub title: String,
    pub body_md: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePostRequest {
    pub topic_id: Uuid,
    pub parent_post_id: Option<Uuid>,
    pub body_md: String,
}
