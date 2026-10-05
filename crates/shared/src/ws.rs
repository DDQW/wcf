//! Realtime protocol spoken over the single multiplexed `/ws` connection.
//! Both sides (de)serialise these enums as JSON.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::chat::ChatMessageView;
use crate::forum::PostView;
use crate::notification::NotificationView;
use crate::reaction::ReactionCount;
use crate::user::UserSummary;

/// Something a client can subscribe to. Authorisation is checked by the
/// server on every subscribe (e.g. private chat channels).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Topic {
    /// Online/offline changes of all users.
    Presence,
    ForumTopic(Uuid),
    Chat(Uuid),
    Watch(Uuid),
    Call(Uuid),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Subscribe { topic: Topic },
    Unsubscribe { topic: Topic },
    /// Clock-sync / keepalive; answered with [`ServerMsg::Pong`].
    Ping { client_ts: f64 },
    /// "I am typing" in a chat channel (sent at most every few seconds).
    Typing { channel_id: Uuid },
    /// Module-specific payloads (watch rooms, calls) as raw JSON.
    Module { module: String, payload: serde_json::Value },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// First message after (re)connecting. Clients use it to resync state
    /// they may have missed while disconnected.
    Hello {
        user_id: Option<Uuid>,
        server_ts: f64,
        online: Vec<Uuid>,
        unread_notifications: i64,
    },
    Pong {
        client_ts: f64,
        server_ts: f64,
    },
    Presence {
        user_id: Uuid,
        online: bool,
    },
    Notification {
        notification: NotificationView,
        unread: i64,
    },
    NotificationsRead {
        unread: i64,
    },
    ForumPostCreated {
        post: PostView,
    },
    ForumPostUpdated {
        post: PostView,
    },
    ForumPostDeleted {
        topic_id: Uuid,
        post_id: Uuid,
    },
    ForumTopicUpdated {
        topic_id: Uuid,
    },
    ChatMessage {
        message: ChatMessageView,
    },
    ChatMessageUpdated {
        message: ChatMessageView,
    },
    ChatMessageDeleted {
        channel_id: Uuid,
        message_id: Uuid,
    },
    ChatReactions {
        channel_id: Uuid,
        message_id: Uuid,
        reactions: Vec<ReactionCount>,
    },
    ChatTyping {
        channel_id: Uuid,
        user: UserSummary,
    },
    /// Sent to every member's own connections when a channel gets a new
    /// message, so sidebars and badges can update.
    ChatActivity {
        channel_id: Uuid,
        mention: bool,
    },
    ChatChannelUpdated {
        channel_id: Uuid,
    },
    /// Module-specific payloads (watch rooms, calls) as raw JSON.
    Module {
        module: String,
        payload: serde_json::Value,
    },
    Error {
        message: String,
    },
}
