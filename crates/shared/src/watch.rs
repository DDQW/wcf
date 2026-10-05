//! Watch together ("Kino"): rooms with a server-authoritative playback clock,
//! a cytu.be-style playlist, scheduled shows with intermissions (ported from
//! HQTube), a media library, room chat with emotes, requests and moderation.
//!
//! Realtime payloads travel as `ServerMsg::Module { module: "watch", .. }`
//! (server → client) and `ClientMsg::Module { module: "watch", .. }`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::user::UserSummary;

pub const MODULE: &str = "watch";
/// Chat history sent on join.
pub const CHAT_HISTORY: i64 = 50;
pub const CHAT_MAX_CHARS: usize = 500;
/// How far ahead of `starts_at` a room enters preshow.
pub const PRESHOW_LEAD_MS: i64 = 10 * 60_000;
pub const OPEN_PROPOSALS_PER_USER: i64 = 3;

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchVisibility {
    /// Every logged-in member can join.
    #[default]
    Members,
    /// Only room members (invited) and moderators.
    Private,
}

impl WatchVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            WatchVisibility::Members => "members",
            WatchVisibility::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "private" => WatchVisibility::Private,
            _ => WatchVisibility::Members,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            WatchVisibility::Members => "Alle Mitglieder",
            WatchVisibility::Private => "Nur Eingeladene",
        }
    }
}

/// Who may add to the playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistMode {
    /// Everyone in the room.
    #[default]
    Open,
    /// Room members (invited) and moderators.
    Members,
    /// Only owner, moderators and the current leader.
    Mods,
}

impl PlaylistMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlaylistMode::Open => "open",
            PlaylistMode::Members => "members",
            PlaylistMode::Mods => "mods",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "members" => PlaylistMode::Members,
            "mods" => PlaylistMode::Mods,
            _ => PlaylistMode::Open,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PlaylistMode::Open => "Offen – alle Zuschauer",
            PlaylistMode::Members => "Raummitglieder",
            PlaylistMode::Mods => "Nur Moderatoren",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomRole {
    Owner,
    Mod,
    Member,
}

impl RoomRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            RoomRole::Owner => "owner",
            RoomRole::Mod => "mod",
            RoomRole::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(RoomRole::Owner),
            "mod" => Some(RoomRole::Mod),
            "member" => Some(RoomRole::Member),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RoomRole::Owner => "Inhaber",
            RoomRole::Mod => "Moderator",
            RoomRole::Member => "Mitglied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowSummary {
    pub title: String,
    pub live: bool,
    pub phase: Option<ShowPhase>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextEvent {
    pub title: String,
    pub starts_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchRoomView {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub owner: UserSummary,
    pub visibility: WatchVisibility,
    pub playlist_mode: PlaylistMode,
    pub slow_mode_ms: i64,
    /// Share of viewers needed to skip (1–100).
    pub voteskip_percent: i32,
    pub viewers: i64,
    pub now_playing: Option<String>,
    pub show: Option<ShowSummary>,
    pub next_event: Option<NextEvent>,
    pub can_edit: bool,
    pub can_control: bool,
}

impl WatchRoomView {
    pub fn url(&self) -> String {
        format!("/watch/{}", self.slug)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchRoomInput {
    pub name: String,
    pub description: String,
    pub visibility: WatchVisibility,
    pub playlist_mode: PlaylistMode,
    pub voteskip_percent: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomRoleRow {
    pub user: UserSummary,
    pub role: RoomRole,
}

// ---------------------------------------------------------------------------
// Media + playback
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Sub,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaTrack {
    pub kind: TrackKind,
    pub lang: Option<String>,
    pub label: String,
    pub track_index: i32,
    pub is_default: bool,
}

/// How the browser plays an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayKind {
    Hls,
    Direct,
    Youtube,
}

/// Everything a client needs to render what is on stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayingMedia {
    /// Stable identifier of this stage item (playlist item or library media).
    pub key: String,
    pub item_id: Option<Uuid>,
    pub media_id: Option<Uuid>,
    pub title: String,
    pub kind: PlayKind,
    /// URL, or the video id for YouTube.
    pub url: String,
    pub duration_ms: Option<i64>,
    pub tracks: Vec<MediaTrack>,
    pub poster_url: Option<String>,
    /// Prefix for side-loaded WebVTT files (`{subs_base}s{track_index}.vtt`).
    pub subs_base: Option<String>,
}

/// Playback state as broadcast by the server-authoritative clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PlaybackState {
    Idle,
    Preshow {
        event_id: Uuid,
        title: String,
        starts_at_ms: i64,
        poster_url: Option<String>,
    },
    Playing {
        media: PlayingMedia,
        /// Position at `at_server_ms`.
        position_ms: f64,
        at_server_ms: f64,
        rate: f64,
    },
    Paused {
        media: PlayingMedia,
        position_ms: f64,
    },
    Intermission {
        event_id: Uuid,
        ends_at_ms: i64,
        next_title: Option<String>,
        next_poster_url: Option<String>,
        item_index: usize,
        item_count: usize,
    },
    Ended {
        event_id: Uuid,
        title: String,
    },
}

impl PlaybackState {
    pub fn media(&self) -> Option<&PlayingMedia> {
        match self {
            PlaybackState::Playing { media, .. } | PlaybackState::Paused { media, .. } => Some(media),
            _ => None,
        }
    }

    /// Room position extrapolated to `server_now_ms`.
    pub fn position_at(&self, server_now_ms: f64) -> f64 {
        match self {
            PlaybackState::Playing { position_ms, at_server_ms, rate, .. } => {
                position_ms + (server_now_ms - at_server_ms) * rate
            }
            PlaybackState::Paused { position_ms, .. } => *position_ms,
            _ => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Shows (scheduled events)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Scheduled,
    Live,
    Ended,
    Cancelled,
}

impl EventStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventStatus::Scheduled => "scheduled",
            EventStatus::Live => "live",
            EventStatus::Ended => "ended",
            EventStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "live" => EventStatus::Live,
            "ended" => EventStatus::Ended,
            "cancelled" => EventStatus::Cancelled,
            _ => EventStatus::Scheduled,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            EventStatus::Scheduled => "geplant",
            EventStatus::Live => "live",
            EventStatus::Ended => "beendet",
            EventStatus::Cancelled => "abgesagt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowPhase {
    Preshow,
    Playing,
    Paused,
    Intermission,
}

impl ShowPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShowPhase::Preshow => "preshow",
            ShowPhase::Playing => "playing",
            ShowPhase::Paused => "paused",
            ShowPhase::Intermission => "intermission",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "preshow" => Some(ShowPhase::Preshow),
            "playing" => Some(ShowPhase::Playing),
            "paused" => Some(ShowPhase::Paused),
            "intermission" => Some(ShowPhase::Intermission),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ShowPhase::Preshow => "Vorprogramm",
            ShowPhase::Playing => "läuft",
            ShowPhase::Paused => "pausiert",
            ShowPhase::Intermission => "Pause",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowItem {
    pub id: Uuid,
    pub media_id: Uuid,
    pub title: String,
    pub duration_ms: Option<i64>,
    pub intermission_after_ms: i64,
    pub poster_url: Option<String>,
    /// False when the media is not packaged (yet).
    pub ready: bool,
}

/// The room's upcoming/running show (banner + console).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowInfo {
    pub event_id: Uuid,
    pub title: String,
    pub status: EventStatus,
    pub starts_at_ms: i64,
    pub items: Vec<ShowItem>,
    pub current_index: Option<usize>,
    pub phase: Option<ShowPhase>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventInfo {
    pub id: Uuid,
    pub room_id: Uuid,
    pub title: String,
    pub starts_at_ms: i64,
    pub status: EventStatus,
    pub phase: Option<ShowPhase>,
    pub item_index: Option<i32>,
    pub items: Vec<ShowItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventItemInput {
    pub media_id: Uuid,
    pub intermission_after_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventInput {
    pub title: String,
    pub starts_at_ms: i64,
    pub items: Vec<EventItemInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSaved {
    pub event: EventInfo,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Library,
    Youtube,
    Url,
    File,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Library => "library",
            SourceKind::Youtube => "youtube",
            SourceKind::Url => "url",
            SourceKind::File => "file",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "library" => SourceKind::Library,
            "youtube" => SourceKind::Youtube,
            "file" => SourceKind::File,
            _ => SourceKind::Url,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Library => "Mediathek",
            SourceKind::Youtube => "YouTube",
            SourceKind::Url => "Link",
            SourceKind::File => "Datei",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub id: Uuid,
    pub title: String,
    pub kind: SourceKind,
    pub duration_ms: Option<i64>,
    pub thumb_url: Option<String>,
    pub added_by: Option<UserSummary>,
}

/// What to add to the playlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaylistSource {
    /// YouTube link or direct video URL (mp4/webm/m3u8).
    Url { url: String },
    Library { media_id: Uuid },
    File { file_id: Uuid },
}

// ---------------------------------------------------------------------------
// Media library
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaStatus {
    New,
    Probing,
    Transcoding,
    Ready,
    Error,
    Archived,
}

impl MediaStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaStatus::New => "new",
            MediaStatus::Probing => "probing",
            MediaStatus::Transcoding => "transcoding",
            MediaStatus::Ready => "ready",
            MediaStatus::Error => "error",
            MediaStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "probing" => MediaStatus::Probing,
            "transcoding" => MediaStatus::Transcoding,
            "ready" => MediaStatus::Ready,
            "error" => MediaStatus::Error,
            "archived" => MediaStatus::Archived,
            _ => MediaStatus::New,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            MediaStatus::New => "Spuren wählen",
            MediaStatus::Probing => "wird vorbereitet",
            MediaStatus::Transcoding => "wird verpackt",
            MediaStatus::Ready => "bereit",
            MediaStatus::Error => "Fehler",
            MediaStatus::Archived => "archiviert",
        }
    }
}

/// Delivery mode chosen at import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// Direct play for eligible single-audio files, HLS otherwise.
    #[default]
    Auto,
    /// Serve the prepared mp4/webm untouched (default audio only).
    Direct,
    /// Lossless remux into multi-audio HLS (transcode only if needed).
    Hls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    Video,
    Audio,
    Sub,
}

/// A stream found by ffprobe, indexed like `-map 0:a:N` / `0:s:N`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeTrack {
    pub kind: ProbeKind,
    pub type_index: i32,
    pub codec: String,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub is_default: bool,
    pub channels: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Subtitles only: text-based subs convert to WebVTT, bitmap subs cannot.
    pub text_based: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub id: Uuid,
    pub title: String,
    pub play_kind: Option<PlayKind>,
    pub status: MediaStatus,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
    /// 0..1 while packaging.
    pub progress: Option<f64>,
    pub poster_url: Option<String>,
    pub tracks: Vec<MediaTrack>,
    /// Admins, HLS media: the source tracks to pick from.
    pub probe: Option<Vec<ProbeTrack>>,
    pub source_name: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportFile {
    pub path: String,
    pub rel_path: String,
    pub size: i64,
    pub suggested_title: String,
    pub imported_as: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryOverview {
    pub media: Vec<MediaInfo>,
    pub import_dir: String,
    pub used_bytes: i64,
    pub cap_bytes: Option<i64>,
    pub ffmpeg: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageSelection {
    pub audio: Vec<i32>,
    pub subs: Vec<i32>,
}

// ---------------------------------------------------------------------------
// Chat, requests, moderation, emotes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchChatMessage {
    pub id: Uuid,
    pub user: UserSummary,
    pub body: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresenceUser {
    pub user: UserSummary,
    pub role: Option<RoomRole>,
    pub leader: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchYou {
    pub role: Option<RoomRole>,
    pub can_control: bool,
    pub can_add: bool,
    pub leader: bool,
    pub muted_until_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Open,
    Scheduled,
    Rejected,
}

impl ProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalStatus::Open => "open",
            ProposalStatus::Scheduled => "scheduled",
            ProposalStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "scheduled" => ProposalStatus::Scheduled,
            "rejected" => ProposalStatus::Rejected,
            _ => ProposalStatus::Open,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProposalStatus::Open => "offen",
            ProposalStatus::Scheduled => "eingeplant",
            ProposalStatus::Rejected => "abgelehnt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalInfo {
    pub id: Uuid,
    pub title: String,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub status: ProposalStatus,
    pub votes: i64,
    pub my_vote: bool,
    pub created_by: UserSummary,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalInput {
    pub title: String,
    pub notes: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationKind {
    Timeout,
    Ban,
    Unban,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestrictedUser {
    pub user: UserSummary,
    pub banned: bool,
    pub until_ms: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emote {
    pub id: Uuid,
    pub code: String,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Realtime protocol
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Play,
    Pause,
    Seek,
    Stop,
    /// Play a library media item right away (outside the playlist).
    PlayNow,
    /// Jump to a playlist item.
    PlayItem,
    /// Skip to the next playlist item.
    Next,
    StartEvent,
    SkipItem,
    ExtendIntermission,
    EndIntermission,
    EndEvent,
    /// Hand the leader role to `user_id` (or clear it).
    SetLeader,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WatchClientMsg {
    Chat {
        room_id: Uuid,
        body: String,
    },
    Control {
        room_id: Uuid,
        action: ControlAction,
        #[serde(default)]
        position_ms: Option<f64>,
        #[serde(default)]
        media_id: Option<Uuid>,
        #[serde(default)]
        item_id: Option<Uuid>,
        #[serde(default)]
        event_id: Option<Uuid>,
        #[serde(default)]
        delta_ms: Option<i64>,
        #[serde(default)]
        user_id: Option<Uuid>,
    },
    Voteskip {
        room_id: Uuid,
    },
    /// A client learned the length of an item the server did not know.
    Duration {
        room_id: Uuid,
        key: String,
        duration_ms: i64,
    },
}

/// Wire type: built and serialised right away, so the size of `Welcome`
/// does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WatchEvent {
    Welcome {
        room: WatchRoomView,
        you: WatchYou,
        state: PlaybackState,
        show: Option<ShowInfo>,
        playlist: Vec<PlaylistItem>,
        current_item: Option<Uuid>,
        chat: Vec<WatchChatMessage>,
        presence: Vec<PresenceUser>,
        voteskip: (usize, usize),
    },
    State {
        room_id: Uuid,
        state: PlaybackState,
    },
    Show {
        room_id: Uuid,
        show: Option<ShowInfo>,
    },
    Playlist {
        room_id: Uuid,
        items: Vec<PlaylistItem>,
        current_item: Option<Uuid>,
    },
    Chat {
        room_id: Uuid,
        msg: WatchChatMessage,
    },
    ChatRemoved {
        room_id: Uuid,
        ids: Vec<Uuid>,
    },
    Presence {
        room_id: Uuid,
        users: Vec<PresenceUser>,
    },
    You {
        room_id: Uuid,
        you: WatchYou,
    },
    SlowMode {
        room_id: Uuid,
        ms: i64,
    },
    /// (votes, needed)
    Voteskip {
        room_id: Uuid,
        votes: usize,
        needed: usize,
    },
    ProposalsChanged {
        room_id: Uuid,
    },
    RoomUpdated {
        room_id: Uuid,
    },
    Kicked {
        room_id: Uuid,
        reason: String,
    },
    Error {
        room_id: Uuid,
        code: String,
        message: String,
    },
}

impl WatchEvent {
    pub fn from_payload(module: &str, payload: &serde_json::Value) -> Option<Self> {
        (module == MODULE).then(|| serde_json::from_value(payload.clone()).ok()).flatten()
    }
}

/// `0:42`, `12:05`, `1:02:33`.
pub fn fmt_timecode(ms: f64) -> String {
    let s = (ms.max(0.0) / 1000.0).floor() as i64;
    let (h, m, sec) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// YouTube video id from the common URL shapes.
pub fn youtube_id(url: &str) -> Option<String> {
    let u = url.trim();
    let rest = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"))
        .unwrap_or(u);
    let rest = rest.strip_prefix("www.").or_else(|| rest.strip_prefix("m.")).unwrap_or(rest);
    let candidate = if let Some(r) = rest.strip_prefix("youtu.be/") {
        r.split(['?', '&', '#', '/']).next()
    } else if let Some(r) = rest.strip_prefix("youtube.com/") {
        if let Some(q) = r.strip_prefix("watch?") {
            q.split('&').find_map(|kv| kv.strip_prefix("v="))
        } else if let Some(x) = r
            .strip_prefix("shorts/")
            .or_else(|| r.strip_prefix("embed/"))
            .or_else(|| r.strip_prefix("live/"))
        {
            x.split(['?', '&', '#', '/']).next()
        } else {
            None
        }
    } else {
        None
    }?;
    let id: String = candidate.chars().take(16).collect();
    (id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_ids() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=10",
            "https://youtu.be/dQw4w9WgXcQ?t=3",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "youtube.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_id(url).as_deref(), Some("dQw4w9WgXcQ"), "{url}");
        }
        assert_eq!(youtube_id("https://example.org/video.mp4"), None);
        assert_eq!(youtube_id("https://youtu.be/short"), None);
    }

    #[test]
    fn timecodes() {
        assert_eq!(fmt_timecode(42_000.0), "0:42");
        assert_eq!(fmt_timecode(725_000.0), "12:05");
        assert_eq!(fmt_timecode(3_753_000.0), "1:02:33");
        assert_eq!(fmt_timecode(-5.0), "0:00");
    }
}
