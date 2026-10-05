use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::user::{NotifyPrefs, UserSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// Someone replied to your forum post or topic.
    Reply,
    /// Someone @mentioned you.
    Mention,
    /// Someone reacted to your content.
    Reaction,
    /// New comment on your blog post / image / entry.
    Comment,
    /// New activity in something you follow (topic, blog).
    Subscription,
    /// New direct message.
    DirectMessage,
    /// Missed call.
    Call,
    /// Moderation outcome (content removed, report handled, ban).
    Moderation,
    /// System announcement.
    System,
}

impl NotificationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NotificationKind::Reply => "reply",
            NotificationKind::Mention => "mention",
            NotificationKind::Reaction => "reaction",
            NotificationKind::Comment => "comment",
            NotificationKind::Subscription => "subscription",
            NotificationKind::DirectMessage => "direct_message",
            NotificationKind::Call => "call",
            NotificationKind::Moderation => "moderation",
            NotificationKind::System => "system",
        }
    }

    pub fn parse(s: &str) -> NotificationKind {
        match s {
            "reply" => NotificationKind::Reply,
            "mention" => NotificationKind::Mention,
            "reaction" => NotificationKind::Reaction,
            "comment" => NotificationKind::Comment,
            "subscription" => NotificationKind::Subscription,
            "direct_message" => NotificationKind::DirectMessage,
            "call" => NotificationKind::Call,
            "moderation" => NotificationKind::Moderation,
            _ => NotificationKind::System,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            NotificationKind::Reply => "reply",
            NotificationKind::Mention => "at-sign",
            NotificationKind::Reaction => "heart",
            NotificationKind::Comment => "message-circle",
            NotificationKind::Subscription => "eye",
            NotificationKind::DirectMessage => "message-square",
            NotificationKind::Call => "phone",
            NotificationKind::Moderation => "shield",
            NotificationKind::System => "info",
        }
    }

    /// Whether the user's preferences allow this kind.
    pub fn enabled_in(&self, prefs: &NotifyPrefs) -> bool {
        match self {
            NotificationKind::Reply => prefs.replies,
            NotificationKind::Mention => prefs.mentions,
            NotificationKind::Reaction => prefs.reactions,
            NotificationKind::Comment => prefs.comments,
            NotificationKind::Subscription => prefs.subscriptions,
            NotificationKind::DirectMessage => prefs.direct_messages,
            NotificationKind::Call => prefs.calls,
            NotificationKind::Moderation => prefs.moderation,
            NotificationKind::System => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationView {
    pub id: Uuid,
    pub kind: NotificationKind,
    pub actor: Option<UserSummary>,
    pub title: String,
    pub body: String,
    pub link: String,
    pub is_read: bool,
    pub created_at: DateTime<Utc>,
}
