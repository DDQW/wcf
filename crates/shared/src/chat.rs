use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::files::FileView;
use crate::reaction::ReactionCount;
use crate::user::UserSummary;

pub const MESSAGES_PER_PAGE: i64 = 50;
pub const MAX_MESSAGE_LEN: usize = 8000;
pub const MAX_ATTACHMENTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Public,
    Private,
    Direct,
    Group,
    /// Chat attached to a watch or call room (not listed in the sidebar).
    Room,
}

impl ChannelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelKind::Public => "public",
            ChannelKind::Private => "private",
            ChannelKind::Direct => "direct",
            ChannelKind::Group => "group",
            ChannelKind::Room => "room",
        }
    }

    pub fn parse(s: &str) -> ChannelKind {
        match s {
            "private" => ChannelKind::Private,
            "direct" => ChannelKind::Direct,
            "group" => ChannelKind::Group,
            "room" => ChannelKind::Room,
            _ => ChannelKind::Public,
        }
    }

    pub fn is_dm(&self) -> bool {
        matches!(self, ChannelKind::Direct | ChannelKind::Group)
    }

    pub fn icon(&self) -> &'static str {
        match self {
            ChannelKind::Public => "hash",
            ChannelKind::Private => "lock",
            ChannelKind::Direct => "user",
            ChannelKind::Group => "users",
            ChannelKind::Room => "tv",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    Owner,
    Admin,
    #[default]
    Member,
}

impl ChatRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChatRole::Owner => "owner",
            ChatRole::Admin => "admin",
            ChatRole::Member => "member",
        }
    }

    pub fn parse(s: &str) -> ChatRole {
        match s {
            "owner" => ChatRole::Owner,
            "admin" => ChatRole::Admin,
            _ => ChatRole::Member,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ChatRole::Owner => "Inhaber",
            ChatRole::Admin => "Admin",
            ChatRole::Member => "Mitglied",
        }
    }

    pub fn can_manage(&self) -> bool {
        matches!(self, ChatRole::Owner | ChatRole::Admin)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelView {
    pub id: Uuid,
    pub kind: ChannelKind,
    /// For DMs: the other participants' names.
    pub name: String,
    pub topic: String,
    pub is_archived: bool,
    pub member_count: i64,
    pub unread: i64,
    pub mentions: i64,
    pub last_message_at: Option<DateTime<Utc>>,
    pub muted: bool,
    pub joined: bool,
    pub my_role: Option<ChatRole>,
    /// The other user of a direct message.
    pub dm_user: Option<UserSummary>,
}

impl ChannelView {
    pub fn url(&self) -> String {
        format!("/chat/{}", self.id)
    }

    pub fn display_name(&self) -> String {
        match self.kind {
            ChannelKind::Public | ChannelKind::Private => format!("#{}", self.name),
            _ => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMemberView {
    pub user: UserSummary,
    pub role: ChatRole,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelDetail {
    pub channel: ChannelView,
    pub members: Vec<ChatMemberView>,
    pub can_post: bool,
    pub can_manage: bool,
    pub can_moderate: bool,
    /// Viewer's read marker when opening (for the "new messages" line).
    pub last_read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Text,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyPreview {
    pub id: Uuid,
    pub author_name: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessageView {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub kind: MessageKind,
    pub author: Option<UserSummary>,
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub deleted: bool,
    pub pinned: bool,
    pub reply_to: Option<ReplyPreview>,
    pub attachments: Vec<FileView>,
    pub reactions: Vec<ReactionCount>,
    pub mentions_me: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessagePage {
    /// Oldest first.
    pub messages: Vec<ChatMessageView>,
    pub older_cursor: Option<String>,
    pub newer_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewChannel {
    pub name: String,
    pub topic: String,
    pub private: bool,
    pub members: Vec<Uuid>,
}

/// Channel names: lowercase, digits, dashes; 2–40 characters.
pub fn normalize_channel_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().trim_start_matches('#').to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            '-' | '_' | ' ' => out.push('-'),
            _ => {}
        }
    }
    let collapsed = out
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    collapsed.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_names() {
        assert_eq!(normalize_channel_name("#Off Topic!"), "off-topic");
        assert_eq!(normalize_channel_name("Grüße"), "gruesse");
        assert_eq!(normalize_channel_name("  a__b  "), "a-b");
    }
}
