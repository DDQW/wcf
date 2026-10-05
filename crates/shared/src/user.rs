use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Site-wide role. Stored as lowercase TEXT in `users.role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Moderator,
    #[default]
    Member,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Admin, Role::Moderator, Role::Member];

    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Moderator => "moderator",
            Role::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "moderator" => Some(Role::Moderator),
            "member" => Some(Role::Member),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Role::Admin => "Admin",
            Role::Moderator => "Moderator",
            Role::Member => "Mitglied",
        }
    }

    /// Higher rank = more privileges; used for "at least moderator" checks.
    pub fn rank(&self) -> u8 {
        match self {
            Role::Admin => 2,
            Role::Moderator => 1,
            Role::Member => 0,
        }
    }

    pub fn at_least(&self, other: Role) -> bool {
        self.rank() >= other.rank()
    }
}

/// Colour scheme preference, applied as `data-theme` on `<html>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Theme {
        match s {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::System,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Theme::System => "Systemeinstellung",
            Theme::Light => "Hell",
            Theme::Dark => "Dunkel",
        }
    }
}

/// The logged-in user as seen by the UI. Re-loaded from the DB on every
/// request, so role changes and bans take effect immediately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub role: Role,
    pub avatar_url: Option<String>,
    pub theme: Theme,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    /// Moderators and admins.
    pub fn is_mod(&self) -> bool {
        self.role.at_least(Role::Moderator)
    }

    pub fn summary(&self) -> UserSummary {
        UserSummary {
            id: self.id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
            avatar_url: self.avatar_url.clone(),
            role: self.role,
        }
    }
}

/// Compact author/user reference embedded in content views.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub role: Role,
}

impl UserSummary {
    pub fn profile_url(&self) -> String {
        format!("/u/{}", self.username)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileStats {
    pub topics: i64,
    pub posts: i64,
    pub blog_posts: i64,
    pub images: i64,
    pub reactions_received: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileView {
    pub user: UserSummary,
    pub bio_html: String,
    pub location: Option<String>,
    pub website: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub banned_until: Option<DateTime<Utc>>,
    pub ban_reason: Option<String>,
    pub is_active: bool,
    pub stats: ProfileStats,
    pub has_blog: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberItem {
    pub user: UserSummary,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub post_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberSort {
    #[default]
    Newest,
    Name,
    Active,
    Posts,
}

/// Per-user notification switches. Every field defaults to `true`, so new
/// categories added later are opt-out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyPrefs {
    pub replies: bool,
    pub mentions: bool,
    pub reactions: bool,
    pub comments: bool,
    pub subscriptions: bool,
    pub direct_messages: bool,
    pub calls: bool,
    pub moderation: bool,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        Self {
            replies: true,
            mentions: true,
            reactions: true,
            comments: true,
            subscriptions: true,
            direct_messages: true,
            calls: true,
            moderation: true,
        }
    }
}

/// Everything the settings page edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsView {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub bio_md: String,
    pub location: String,
    pub website: String,
    pub theme: Theme,
    pub notify_prefs: NotifyPrefs,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileUpdate {
    pub display_name: String,
    pub bio_md: String,
    pub location: String,
    pub website: String,
}

/// One entry of a user's activity stream (profile page).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityItem {
    pub icon: String,
    pub label: String,
    pub title: String,
    pub excerpt: String,
    pub link: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberPage {
    pub items: Vec<MemberItem>,
    pub total: i64,
    pub per_page: i64,
}
