use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::files::Visibility;
use crate::reaction::ReactionCount;
use crate::user::UserSummary;

pub const ALBUMS_PER_PAGE: i64 = 24;
pub const IMAGES_PER_PAGE: i64 = 48;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumView {
    pub id: Uuid,
    pub owner: UserSummary,
    pub title: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub visibility: Visibility,
    pub is_community: bool,
    pub image_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub can_edit: bool,
    pub can_upload: bool,
}

impl AlbumView {
    pub fn url(&self) -> String {
        format!("/gallery/album/{}", self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumInput {
    pub title: String,
    pub description: String,
    pub visibility: Visibility,
    pub is_community: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageView {
    pub id: Uuid,
    pub album_id: Uuid,
    pub owner: UserSummary,
    pub file_id: Uuid,
    pub title: String,
    pub description: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub taken_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub view_count: i32,
    pub like_count: i64,
    pub comment_count: i64,
    /// Camera details, already labelled (label, value).
    pub exif: Vec<(String, String)>,
    pub file_name: String,
    pub size_bytes: i64,
}

impl ImageView {
    pub fn thumb_url(&self) -> String {
        format!("/files/{}/thumb", self.file_id)
    }

    pub fn preview_url(&self) -> String {
        format!("/files/{}/preview", self.file_id)
    }

    pub fn original_url(&self) -> String {
        format!("/files/{}", self.file_id)
    }

    pub fn download_url(&self) -> String {
        format!("/files/{}/download", self.file_id)
    }

    pub fn url(&self) -> String {
        format!("/gallery/image/{}", self.id)
    }

    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() {
            self.file_name.clone()
        } else {
            self.title.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumDetail {
    pub album: AlbumView,
    pub images: Vec<ImageView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageDetail {
    pub image: ImageView,
    pub album: AlbumView,
    pub prev: Option<Uuid>,
    pub next: Option<Uuid>,
    pub reactions: Vec<ReactionCount>,
    pub can_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlbumFilter {
    All,
    User(Uuid),
    Mine,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumPage {
    pub items: Vec<AlbumView>,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImagePage {
    pub items: Vec<ImageView>,
    pub next_cursor: Option<String>,
}
