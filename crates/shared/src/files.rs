use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::user::UserSummary;

/// Who may see a file, folder, album or room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Members,
    Public,
}

impl Visibility {
    pub const ALL: [Visibility; 3] = [Visibility::Private, Visibility::Members, Visibility::Public];

    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Members => "members",
            Visibility::Public => "public",
        }
    }

    pub fn parse(s: &str) -> Visibility {
        match s {
            "members" => Visibility::Members,
            "public" => Visibility::Public,
            _ => Visibility::Private,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Visibility::Private => "Privat",
            Visibility::Members => "Mitglieder",
            Visibility::Public => "Öffentlich",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Visibility::Private => "lock",
            Visibility::Members => "users",
            Visibility::Public => "globe",
        }
    }
}

/// What an upload is used for; non-drive files are hidden from the drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilePurpose {
    #[default]
    Drive,
    Avatar,
    /// Image embedded in markdown (forum, blog, profile).
    Embed,
    Chat,
    Gallery,
    Blog,
    Cms,
    Watch,
}

impl FilePurpose {
    pub fn as_str(&self) -> &'static str {
        match self {
            FilePurpose::Drive => "drive",
            FilePurpose::Avatar => "avatar",
            FilePurpose::Embed => "embed",
            FilePurpose::Chat => "chat",
            FilePurpose::Gallery => "gallery",
            FilePurpose::Blog => "blog",
            FilePurpose::Cms => "cms",
            FilePurpose::Watch => "watch",
        }
    }

    pub fn parse(s: &str) -> FilePurpose {
        match s {
            "avatar" => FilePurpose::Avatar,
            "embed" => FilePurpose::Embed,
            "chat" => FilePurpose::Chat,
            "gallery" => FilePurpose::Gallery,
            "blog" => FilePurpose::Blog,
            "cms" => FilePurpose::Cms,
            "watch" => FilePurpose::Watch,
            _ => FilePurpose::Drive,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            FilePurpose::Drive => "Dateien",
            FilePurpose::Avatar => "Avatar",
            FilePurpose::Embed => "Eingebettet",
            FilePurpose::Chat => "Chat-Anhang",
            FilePurpose::Gallery => "Galerie",
            FilePurpose::Blog => "Blog",
            FilePurpose::Cms => "Inhalte",
            FilePurpose::Watch => "Watch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Video,
    Audio,
    Pdf,
    Text,
    Archive,
    Document,
    Other,
}

impl FileKind {
    pub fn of(mime: &str) -> FileKind {
        let m = mime.to_ascii_lowercase();
        if m.starts_with("image/") {
            FileKind::Image
        } else if m.starts_with("video/") {
            FileKind::Video
        } else if m.starts_with("audio/") {
            FileKind::Audio
        } else if m == "application/pdf" {
            FileKind::Pdf
        } else if m.starts_with("text/") || m == "application/json" || m == "application/xml" {
            FileKind::Text
        } else if m.contains("zip") || m.contains("tar") || m.contains("rar") || m.contains("7z") || m.contains("gzip") {
            FileKind::Archive
        } else if m.contains("word") || m.contains("officedocument") || m.contains("opendocument") || m.contains("rtf") {
            FileKind::Document
        } else {
            FileKind::Other
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            FileKind::Image => "image",
            FileKind::Video => "film",
            FileKind::Audio => "music",
            FileKind::Pdf | FileKind::Document | FileKind::Text => "file-text",
            FileKind::Archive => "archive",
            FileKind::Other => "file",
        }
    }

    /// Whether the browser can show it inline in our preview dialog.
    pub fn previewable(&self) -> bool {
        matches!(self, FileKind::Image | FileKind::Video | FileKind::Audio | FileKind::Pdf | FileKind::Text)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileView {
    pub id: Uuid,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_secs: Option<f64>,
    pub has_thumb: bool,
    pub visibility: Visibility,
    pub purpose: FilePurpose,
    pub folder_id: Option<Uuid>,
    pub description: String,
    pub download_count: i32,
    pub owner: UserSummary,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl FileView {
    pub fn kind(&self) -> FileKind {
        FileKind::of(&self.mime_type)
    }

    /// Inline URL (images, video streaming, PDF).
    pub fn url(&self) -> String {
        format!("/files/{}", self.id)
    }

    pub fn download_url(&self) -> String {
        format!("/files/{}/download", self.id)
    }

    pub fn thumb_url(&self) -> Option<String> {
        self.has_thumb.then(|| format!("/files/{}/thumb", self.id))
    }

    /// Large rendition (≤1600px) for images, falling back to the original.
    pub fn preview_url(&self) -> String {
        if self.has_thumb && self.kind() == FileKind::Image {
            format!("/files/{}/preview", self.id)
        } else {
            self.url()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FolderView {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub item_count: i64,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageUsage {
    pub used_bytes: i64,
    /// `None` = unlimited.
    pub quota_bytes: Option<i64>,
    pub trash_bytes: i64,
    pub file_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveSort {
    #[default]
    Name,
    Newest,
    Size,
    Type,
}

impl DriveSort {
    pub fn as_str(&self) -> &'static str {
        match self {
            DriveSort::Name => "name",
            DriveSort::Newest => "newest",
            DriveSort::Size => "size",
            DriveSort::Type => "type",
        }
    }

    pub fn parse(s: &str) -> DriveSort {
        match s {
            "newest" => DriveSort::Newest,
            "size" => DriveSort::Size,
            "type" => DriveSort::Type,
            _ => DriveSort::Name,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DriveSort::Name => "Name",
            DriveSort::Newest => "Neueste",
            DriveSort::Size => "Größe",
            DriveSort::Type => "Typ",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveListing {
    pub folder: Option<FolderView>,
    pub breadcrumbs: Vec<FolderRef>,
    pub folders: Vec<FolderView>,
    pub files: Vec<FileView>,
    pub usage: StorageUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareView {
    pub id: Uuid,
    pub token: String,
    pub file: Option<FolderRef>,
    pub folder: Option<FolderRef>,
    pub expires_at: Option<DateTime<Utc>>,
    pub has_password: bool,
    pub max_downloads: Option<i32>,
    pub download_count: i32,
    pub created_at: DateTime<Utc>,
}

impl ShareView {
    pub fn path(&self) -> String {
        format!("/s/{}", self.token)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareInfo {
    pub token: String,
    pub owner: UserSummary,
    /// The share is password protected and not yet unlocked in this session.
    pub locked: bool,
    pub file: Option<FileView>,
    pub folder: Option<FolderView>,
    pub files: Vec<FileView>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Response of `POST /api/files`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UploadResult {
    pub files: Vec<FileView>,
    pub errors: Vec<String>,
}
