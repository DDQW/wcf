use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reaction::ReactionCount;
use crate::user::UserSummary;

pub const POSTS_PER_PAGE: i64 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostStatus {
    #[default]
    Draft,
    Published,
    /// Published with a date in the future.
    Scheduled,
}

impl PostStatus {
    pub fn label(&self) -> &'static str {
        match self {
            PostStatus::Draft => "Entwurf",
            PostStatus::Published => "Veröffentlicht",
            PostStatus::Scheduled => "Geplant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlogView {
    pub id: Uuid,
    pub owner: UserSummary,
    pub title: String,
    pub description: String,
    pub post_count: i64,
    pub subscriber_count: i64,
    pub subscribed: bool,
    pub created_at: DateTime<Utc>,
    pub last_post_at: Option<DateTime<Utc>>,
}

impl BlogView {
    pub fn url(&self) -> String {
        format!("/blog/{}", self.owner.username)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlogPostItem {
    pub id: Uuid,
    pub blog_id: Uuid,
    pub blog_title: String,
    pub author: UserSummary,
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    pub cover_url: Option<String>,
    pub tags: Vec<String>,
    pub status: PostStatus,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reading_minutes: i32,
    pub view_count: i32,
    pub comment_count: i64,
    pub like_count: i64,
}

impl BlogPostItem {
    pub fn url(&self) -> String {
        format!("/blog/{}/{}", self.author.username, self.slug)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostNeighbour {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlogPostDetail {
    pub post: BlogPostItem,
    pub blog: BlogView,
    pub body_html: String,
    pub allow_comments: bool,
    pub can_edit: bool,
    pub reactions: Vec<ReactionCount>,
    pub newer: Option<PostNeighbour>,
    pub older: Option<PostNeighbour>,
}

/// Editor payload (create when `id` is `None`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlogPostEdit {
    pub id: Option<Uuid>,
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    pub body_md: String,
    pub cover_file_id: Option<Uuid>,
    pub cover_url: Option<String>,
    pub tags: Vec<String>,
    /// `true` = published (immediately or at `publish_at`).
    pub publish: bool,
    pub publish_at: Option<DateTime<Utc>>,
    pub allow_comments: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlogFilter {
    All,
    Blog(Uuid),
    Tag(String),
    /// Blogs the viewer subscribed to.
    Following,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlogPostPage {
    pub items: Vec<BlogPostItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPost {
    pub id: Uuid,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Generic comments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentView {
    pub id: Uuid,
    pub author: Option<UserSummary>,
    pub parent_id: Option<Uuid>,
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub deleted: bool,
    pub reactions: Vec<ReactionCount>,
    /// The viewer may edit (author) or delete (author, subject owner, staff).
    pub can_edit: bool,
    pub can_delete: bool,
}
