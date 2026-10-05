use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::user::Role;

/// Feature modules that admins can switch on/off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModuleToggles {
    pub forum: bool,
    pub chat: bool,
    pub blogs: bool,
    pub gallery: bool,
    pub files: bool,
    pub calls: bool,
    pub watch: bool,
    pub cms: bool,
}

impl Default for ModuleToggles {
    fn default() -> Self {
        Self {
            forum: true,
            chat: true,
            blogs: true,
            gallery: true,
            files: true,
            calls: true,
            watch: true,
            cms: true,
        }
    }
}

/// Admin-editable site configuration, stored as one JSONB row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteSettings {
    pub site_name: String,
    pub site_description: String,
    pub registration_open: bool,
    /// Markdown shown on the start page.
    pub welcome_md: String,
    pub modules: ModuleToggles,
    /// Maximum size of a single upload.
    pub max_upload_mb: i64,
    /// Default storage quota per user (0 = unlimited).
    pub default_quota_mb: i64,
    /// Whether members (not only moderators) may create chat channels.
    pub members_create_channels: bool,
    /// Whether members may create watch-together rooms.
    pub members_create_watch_rooms: bool,
    /// Whether members may create persistent call rooms.
    pub members_create_call_rooms: bool,
}

impl Default for SiteSettings {
    fn default() -> Self {
        Self {
            site_name: "Woelki Community".into(),
            site_description: "Forum, Chat, Blogs, Galerie und mehr".into(),
            registration_open: true,
            welcome_md: "Willkommen in der Community! Schau dich im Forum um, stell dich vor \
                         oder starte einen Chat."
                .into(),
            modules: ModuleToggles::default(),
            max_upload_mb: 512,
            default_quota_mb: 5120,
            members_create_channels: true,
            members_create_watch_rooms: true,
            members_create_call_rooms: true,
        }
    }
}

/// The parts of the site configuration every visitor may see.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicSiteInfo {
    pub site_name: String,
    pub site_description: String,
    pub registration_open: bool,
    pub welcome_html: String,
    pub modules: ModuleToggles,
    pub max_upload_mb: i64,
    pub calls_configured: bool,
    pub password_reset_by_mail: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminStats {
    pub users: i64,
    pub users_new_7d: i64,
    pub online_now: i64,
    pub topics: i64,
    pub posts: i64,
    pub chat_messages: i64,
    pub blog_posts: i64,
    pub images: i64,
    pub files: i64,
    pub storage_bytes: i64,
    pub open_reports: i64,
    pub cms_entries: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminUserRow {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub role: Role,
    pub is_active: bool,
    pub banned_until: Option<DateTime<Utc>>,
    pub ban_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub storage_used: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminUserPage {
    pub items: Vec<AdminUserRow>,
    pub total: i64,
    pub per_page: i64,
}
