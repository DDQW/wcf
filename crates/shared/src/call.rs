//! Calls: rooms, 1:1 calls and the signalling payloads that travel as
//! `ServerMsg::Module { module: "call", .. }` over the realtime socket.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::user::UserSummary;

pub const MODULE: &str = "call";
/// Seconds a 1:1 call rings before it counts as missed.
pub const RING_SECONDS: i64 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomVisibility {
    /// Every logged-in member can see and join.
    #[default]
    Members,
    /// Only invited members (and moderators).
    Private,
}

impl RoomVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            RoomVisibility::Members => "members",
            RoomVisibility::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "private" => RoomVisibility::Private,
            _ => RoomVisibility::Members,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RoomVisibility::Members => "Alle Mitglieder",
            RoomVisibility::Private => "Nur Eingeladene",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallRoomView {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub owner: UserSummary,
    pub visibility: RoomVisibility,
    pub persistent: bool,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    /// Users currently in the call.
    pub participants: Vec<UserSummary>,
    pub can_edit: bool,
    pub can_moderate: bool,
}

impl CallRoomView {
    pub fn url(&self) -> String {
        format!("/calls/{}", self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallRoomInput {
    pub name: String,
    pub description: String,
    pub visibility: RoomVisibility,
    pub persistent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomMember {
    pub user: UserSummary,
    pub moderator: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallRoomDetail {
    pub room: CallRoomView,
    pub members: Vec<RoomMember>,
    pub chat_channel_id: Option<Uuid>,
}

/// Everything the browser needs to connect to the media server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinInfo {
    pub url: String,
    pub token: String,
    pub identity: String,
    pub moderator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    Ringing,
    Accepted,
    Declined,
    Missed,
    Cancelled,
    Ended,
}

impl CallStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallStatus::Ringing => "ringing",
            CallStatus::Accepted => "accepted",
            CallStatus::Declined => "declined",
            CallStatus::Missed => "missed",
            CallStatus::Cancelled => "cancelled",
            CallStatus::Ended => "ended",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "accepted" => CallStatus::Accepted,
            "declined" => CallStatus::Declined,
            "missed" => CallStatus::Missed,
            "cancelled" => CallStatus::Cancelled,
            "ended" => CallStatus::Ended,
            _ => CallStatus::Ringing,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            CallStatus::Ringing => "Klingelt",
            CallStatus::Accepted => "Läuft",
            CallStatus::Declined => "Abgelehnt",
            CallStatus::Missed => "Verpasst",
            CallStatus::Cancelled => "Abgebrochen",
            CallStatus::Ended => "Beendet",
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self, CallStatus::Ringing | CallStatus::Accepted)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectCallView {
    pub id: Uuid,
    pub caller: UserSummary,
    pub callee: UserSummary,
    pub video: bool,
    pub status: CallStatus,
    pub created_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl DirectCallView {
    pub fn url(&self) -> String {
        format!("/calls/direct/{}", self.id)
    }

    /// The other party from `me`'s perspective.
    pub fn other(&self, me: Uuid) -> &UserSummary {
        if self.caller.id == me {
            &self.callee
        } else {
            &self.caller
        }
    }

    pub fn duration_secs(&self) -> Option<i64> {
        match (self.answered_at, self.ended_at) {
            (Some(a), Some(e)) => Some((e - a).num_seconds().max(0)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectCallDetail {
    pub call: DirectCallView,
    /// Present while the call is open and the media server is configured.
    pub join: Option<JoinInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallsOverview {
    pub rooms: Vec<CallRoomView>,
    pub history: Vec<DirectCallView>,
    pub configured: bool,
    pub can_create: bool,
}

/// Signalling events (server → client).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CallEvent {
    /// Someone is calling you.
    Incoming { call: DirectCallView },
    /// The state of a 1:1 call changed (accepted elsewhere, declined …).
    Status { call: DirectCallView },
    /// Live participant list of a room (topic `Call(room_id)` and directory).
    Participants { room_id: Uuid, participants: Vec<UserSummary> },
    /// A moderator removed you from the room.
    Removed { room_id: Uuid, by: UserSummary },
    /// A moderator asked you to mute your microphone.
    MuteRequest { room_id: Uuid, by: UserSummary },
    RoomUpdated { room_id: Uuid },
    RoomDeleted { room_id: Uuid },
}

impl CallEvent {
    pub fn from_payload(module: &str, payload: &serde_json::Value) -> Option<Self> {
        (module == MODULE).then(|| serde_json::from_value(payload.clone()).ok()).flatten()
    }
}
