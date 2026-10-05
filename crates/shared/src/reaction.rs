use serde::{Deserialize, Serialize};

/// Fixed reaction palette — arbitrary strings are rejected server-side.
pub const REACTIONS: [&str; 8] = ["👍", "❤️", "😂", "😮", "😢", "🎉", "🙏", "👀"];

pub fn is_valid_reaction(emoji: &str) -> bool {
    REACTIONS.contains(&emoji)
}

/// Content types that can be reacted to, reported, commented on or searched.
/// Stored as snake_case TEXT (`subject_type` columns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectType {
    ForumTopic,
    ForumPost,
    ChatMessage,
    BlogPost,
    Comment,
    GalleryImage,
    GalleryAlbum,
    File,
    CmsEntry,
    User,
}

impl SubjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubjectType::ForumTopic => "forum_topic",
            SubjectType::ForumPost => "forum_post",
            SubjectType::ChatMessage => "chat_message",
            SubjectType::BlogPost => "blog_post",
            SubjectType::Comment => "comment",
            SubjectType::GalleryImage => "gallery_image",
            SubjectType::GalleryAlbum => "gallery_album",
            SubjectType::File => "file",
            SubjectType::CmsEntry => "cms_entry",
            SubjectType::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<SubjectType> {
        Some(match s {
            "forum_topic" => SubjectType::ForumTopic,
            "forum_post" => SubjectType::ForumPost,
            "chat_message" => SubjectType::ChatMessage,
            "blog_post" => SubjectType::BlogPost,
            "comment" => SubjectType::Comment,
            "gallery_image" => SubjectType::GalleryImage,
            "gallery_album" => SubjectType::GalleryAlbum,
            "file" => SubjectType::File,
            "cms_entry" => SubjectType::CmsEntry,
            "user" => SubjectType::User,
            _ => return None,
        })
    }

    pub fn label(&self) -> &'static str {
        match self {
            SubjectType::ForumTopic => "Forenthema",
            SubjectType::ForumPost => "Forenbeitrag",
            SubjectType::ChatMessage => "Chatnachricht",
            SubjectType::BlogPost => "Blogbeitrag",
            SubjectType::Comment => "Kommentar",
            SubjectType::GalleryImage => "Bild",
            SubjectType::GalleryAlbum => "Album",
            SubjectType::File => "Datei",
            SubjectType::CmsEntry => "Inhalt",
            SubjectType::User => "Benutzer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionCount {
    pub emoji: String,
    pub count: i64,
    /// Whether the viewing user is among the reactors.
    pub me: bool,
}
