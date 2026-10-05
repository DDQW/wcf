use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::user::UserSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    ForumTopic,
    ForumPost,
    BlogPost,
    ChatMessage,
    File,
    GalleryImage,
    CmsEntry,
    User,
}

impl SearchKind {
    pub const ALL: [SearchKind; 8] = [
        SearchKind::ForumTopic,
        SearchKind::ForumPost,
        SearchKind::BlogPost,
        SearchKind::ChatMessage,
        SearchKind::GalleryImage,
        SearchKind::File,
        SearchKind::CmsEntry,
        SearchKind::User,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            SearchKind::ForumTopic => "forum_topic",
            SearchKind::ForumPost => "forum_post",
            SearchKind::BlogPost => "blog_post",
            SearchKind::ChatMessage => "chat_message",
            SearchKind::File => "file",
            SearchKind::GalleryImage => "gallery_image",
            SearchKind::CmsEntry => "cms_entry",
            SearchKind::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<SearchKind> {
        SearchKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn label(&self) -> &'static str {
        match self {
            SearchKind::ForumTopic => "Themen",
            SearchKind::ForumPost => "Beiträge",
            SearchKind::BlogPost => "Blogs",
            SearchKind::ChatMessage => "Chat",
            SearchKind::File => "Dateien",
            SearchKind::GalleryImage => "Bilder",
            SearchKind::CmsEntry => "Seiten & Inhalte",
            SearchKind::User => "Mitglieder",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            SearchKind::ForumTopic => "message-square",
            SearchKind::ForumPost => "message-circle",
            SearchKind::BlogPost => "book-open",
            SearchKind::ChatMessage => "hash",
            SearchKind::File => "file",
            SearchKind::GalleryImage => "image",
            SearchKind::CmsEntry => "layout",
            SearchKind::User => "user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub kind: SearchKind,
    pub title: String,
    /// HTML with `<mark>` highlights produced by `ts_headline` (sanitised).
    pub snippet_html: String,
    pub link: String,
    pub author: Option<UserSummary>,
    pub created_at: Option<DateTime<Utc>>,
    pub thumb_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Hit counts per kind (for the filter tabs).
    pub counts: Vec<(SearchKind, i64)>,
}
