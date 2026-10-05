use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reaction::ReactionCount;
use crate::user::{Role, UserSummary};

/// Number of thread blocks per page in a topic.
pub const BLOCKS_PER_PAGE: i64 = 10;
/// Replies shipped inline with each block; the rest load on demand.
pub const REPLIES_PER_BLOCK: i64 = 30;
/// Topics per page in topic lists.
pub const TOPICS_PER_PAGE: i64 = 25;
pub const MAX_TAGS_PER_TOPIC: usize = 5;

/// Default palette offered in the category editor.
pub const CATEGORY_COLORS: [&str; 10] = [
    "#2563eb", "#7c3aed", "#db2777", "#dc2626", "#ea580c", "#ca8a04", "#16a34a", "#0d9488",
    "#0891b2", "#64748b",
];

/// Read access of a forum category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryAccess {
    /// Everyone, including visitors who are not logged in.
    #[default]
    Public,
    /// Logged-in members only.
    Members,
    /// Moderators and admins only.
    Staff,
}

impl CategoryAccess {
    pub const ALL: [CategoryAccess; 3] =
        [CategoryAccess::Public, CategoryAccess::Members, CategoryAccess::Staff];

    pub fn as_str(&self) -> &'static str {
        match self {
            CategoryAccess::Public => "public",
            CategoryAccess::Members => "members",
            CategoryAccess::Staff => "staff",
        }
    }

    pub fn parse(s: &str) -> CategoryAccess {
        match s {
            "members" => CategoryAccess::Members,
            "staff" => CategoryAccess::Staff,
            _ => CategoryAccess::Public,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            CategoryAccess::Public => "Öffentlich",
            CategoryAccess::Members => "Nur Mitglieder",
            CategoryAccess::Staff => "Nur Moderation",
        }
    }

    /// Whether a viewer with the given role (None = guest) may read.
    pub fn allows(&self, role: Option<Role>) -> bool {
        match self {
            CategoryAccess::Public => true,
            CategoryAccess::Members => role.is_some(),
            CategoryAccess::Staff => role.is_some_and(|r| r.at_least(Role::Moderator)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastTopicInfo {
    pub id: Uuid,
    pub title: String,
    pub last_post_at: DateTime<Utc>,
    pub last_poster: Option<UserSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategoryView {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub color: String,
    pub position: i32,
    pub parent_id: Option<Uuid>,
    /// Who may read the category.
    pub read_access: CategoryAccess,
    /// Minimum role needed to start topics.
    pub post_role: Role,
    pub topic_count: i64,
    pub post_count: i64,
    pub last_topic: Option<LastTopicInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategoryInput {
    pub name: String,
    pub description: String,
    pub color: String,
    pub parent_id: Option<Uuid>,
    pub read_access: CategoryAccess,
    pub post_role: Role,
    pub position: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryRef {
    pub id: Uuid,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicSort {
    #[default]
    Activity,
    Newest,
    Top,
}

impl TopicSort {
    pub fn as_str(&self) -> &'static str {
        match self {
            TopicSort::Activity => "activity",
            TopicSort::Newest => "newest",
            TopicSort::Top => "top",
        }
    }

    pub fn parse(s: &str) -> TopicSort {
        match s {
            "newest" => TopicSort::Newest,
            "top" => TopicSort::Top,
            _ => TopicSort::Activity,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            TopicSort::Activity => "Aktivität",
            TopicSort::Newest => "Neu",
            TopicSort::Top => "Top",
        }
    }
}

/// Which topics a topic list shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopicFilter {
    All,
    Category(Uuid),
    Tag(String),
    Author(Uuid),
    /// Topics the viewer watches.
    Watching,
    /// Topics with unread posts for the viewer.
    Unread,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicListItem {
    pub id: Uuid,
    pub category: CategoryRef,
    pub title: String,
    pub slug: String,
    pub author: UserSummary,
    pub tags: Vec<String>,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub is_deleted: bool,
    pub reply_count: i32,
    pub view_count: i32,
    pub like_count: i64,
    pub last_post_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub last_poster: Option<UserSummary>,
    /// `None` = viewer never opened the topic (or anonymous).
    pub unread_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicPage {
    pub items: Vec<TopicListItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionLevel {
    /// Notify on every new post.
    Watching,
    /// Only direct replies and mentions.
    #[default]
    Normal,
    /// Nothing except direct mentions.
    Muted,
}

impl SubscriptionLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubscriptionLevel::Watching => "watching",
            SubscriptionLevel::Normal => "normal",
            SubscriptionLevel::Muted => "muted",
        }
    }

    pub fn parse(s: &str) -> SubscriptionLevel {
        match s {
            "watching" => SubscriptionLevel::Watching,
            "muted" => SubscriptionLevel::Muted,
            _ => SubscriptionLevel::Normal,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SubscriptionLevel::Watching => "Beobachten",
            SubscriptionLevel::Normal => "Normal",
            SubscriptionLevel::Muted => "Stumm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicDetail {
    pub id: Uuid,
    pub category: CategoryRef,
    pub title: String,
    pub slug: String,
    pub author: UserSummary,
    pub tags: Vec<String>,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub is_deleted: bool,
    pub reply_count: i32,
    pub view_count: i32,
    pub created_at: DateTime<Utc>,
    pub last_post_at: DateTime<Utc>,
    pub subscription: SubscriptionLevel,
    /// Viewer's read marker *before* this visit — posts newer than this are
    /// highlighted as new. `None` for first visits and anonymous viewers.
    pub last_read_at: Option<DateTime<Utc>>,
    /// First post the viewer has not seen yet (for "jump to unread").
    pub first_unread_post: Option<Uuid>,
    /// The opening post (deleting it deletes the topic).
    pub first_post_id: Option<Uuid>,
    pub can_reply: bool,
    pub can_moderate: bool,
    pub can_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostView {
    pub id: Uuid,
    pub topic_id: Uuid,
    pub author: UserSummary,
    pub parent_post_id: Option<Uuid>,
    pub thread_root_id: Uuid,
    pub depth: i16,
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub edit_count: i32,
    pub is_deleted: bool,
    pub reactions: Vec<ReactionCount>,
    /// Display name of the author of the parent post ("Antwort an …").
    pub reply_to: Option<String>,
}

/// A top-level post with (the first page of) all replies in its thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadBlock {
    pub root: PostView,
    pub replies: Vec<PostView>,
    pub reply_total: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockPage {
    pub blocks: Vec<ThreadBlock>,
    pub next_cursor: Option<String>,
    /// Set when blocks exist before this page (after jumping to a post).
    pub prev_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyPage {
    pub replies: Vec<PostView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostRevision {
    pub id: Uuid,
    pub body_html: String,
    pub editor: UserSummary,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewTopic {
    pub category_id: Uuid,
    pub title: String,
    pub body_md: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicUpdate {
    pub title: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForumOverview {
    pub categories: Vec<CategoryView>,
    pub popular_tags: Vec<TagCount>,
}

/// Normalises user-entered tags: lowercase, trimmed, deduplicated,
/// limited to letters/digits/dashes and [`MAX_TAGS_PER_TOPIC`] entries.
pub fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw {
        let cleaned: String = t
            .trim()
            .trim_start_matches('#')
            .to_lowercase()
            .chars()
            .map(|c| if c.is_whitespace() || c == '_' { '-' } else { c })
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .take(24)
            .collect();
        let cleaned = cleaned.trim_matches('-').to_string();
        if !cleaned.is_empty() && !out.contains(&cleaned) {
            out.push(cleaned);
        }
        if out.len() >= MAX_TAGS_PER_TOPIC {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_normalized() {
        let tags = normalize_tags(&[
            " Rust ".into(),
            "#rust".into(),
            "Web Dev".into(),
            "!!".into(),
            "Größe".into(),
        ]);
        assert_eq!(tags, vec!["rust", "web-dev", "größe"]);
    }

    #[test]
    fn tags_are_limited() {
        let raw: Vec<String> = (0..10).map(|i| format!("t{i}")).collect();
        assert_eq!(normalize_tags(&raw).len(), MAX_TAGS_PER_TOPIC);
    }
}
