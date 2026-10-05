//! Realtime protocol spoken over the single multiplexed `/ws` connection.
//! Both sides (de)serialise these enums as JSON.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::forum::PostView;
use crate::notification::NotificationView;

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
    Error {
        message: String,
    },
}
