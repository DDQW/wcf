use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reaction::SubjectType;
use crate::user::UserSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportReason {
    Spam,
    Harassment,
    OffTopic,
    Illegal,
    Inappropriate,
    Other,
}

impl ReportReason {
    pub const ALL: [ReportReason; 6] = [
        ReportReason::Spam,
        ReportReason::Harassment,
        ReportReason::OffTopic,
        ReportReason::Inappropriate,
        ReportReason::Illegal,
        ReportReason::Other,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ReportReason::Spam => "spam",
            ReportReason::Harassment => "harassment",
            ReportReason::OffTopic => "off_topic",
            ReportReason::Illegal => "illegal",
            ReportReason::Inappropriate => "inappropriate",
            ReportReason::Other => "other",
        }
    }

    pub fn parse(s: &str) -> ReportReason {
        match s {
            "spam" => ReportReason::Spam,
            "harassment" => ReportReason::Harassment,
            "off_topic" => ReportReason::OffTopic,
            "illegal" => ReportReason::Illegal,
            "inappropriate" => ReportReason::Inappropriate,
            _ => ReportReason::Other,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ReportReason::Spam => "Spam / Werbung",
            ReportReason::Harassment => "Belästigung / Beleidigung",
            ReportReason::OffTopic => "Themenfremd",
            ReportReason::Illegal => "Rechtswidrig",
            ReportReason::Inappropriate => "Unangemessen",
            ReportReason::Other => "Sonstiges",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    #[default]
    Open,
    Resolved,
    Dismissed,
}

impl ReportStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReportStatus::Open => "open",
            ReportStatus::Resolved => "resolved",
            ReportStatus::Dismissed => "dismissed",
        }
    }

    pub fn parse(s: &str) -> ReportStatus {
        match s {
            "resolved" => ReportStatus::Resolved,
            "dismissed" => ReportStatus::Dismissed,
            _ => ReportStatus::Open,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ReportStatus::Open => "Offen",
            ReportStatus::Resolved => "Erledigt",
            ReportStatus::Dismissed => "Verworfen",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportView {
    pub id: Uuid,
    pub subject_type: SubjectType,
    pub subject_id: Uuid,
    /// Short text preview of the reported content (may be empty if deleted).
    pub preview: String,
    /// Where a moderator can look at the content in context.
    pub link: Option<String>,
    /// Author of the reported content, if known.
    pub content_author: Option<UserSummary>,
    pub reason: ReportReason,
    pub details: String,
    pub reporter: UserSummary,
    pub status: ReportStatus,
    pub resolved_by: Option<UserSummary>,
    pub resolution_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModLogEntry {
    pub id: Uuid,
    pub actor: Option<UserSummary>,
    pub action: String,
    pub subject_type: Option<String>,
    pub subject_id: Option<Uuid>,
    pub target_user: Option<UserSummary>,
    pub details: String,
    pub created_at: DateTime<Utc>,
}

/// Human-readable label for a mod-log action key.
pub fn mod_action_label(action: &str) -> &str {
    match action {
        "ban" => "Benutzer gesperrt",
        "unban" => "Sperre aufgehoben",
        "role" => "Rolle geändert",
        "deactivate" => "Konto deaktiviert",
        "activate" => "Konto aktiviert",
        "delete_content" => "Inhalt gelöscht",
        "restore_content" => "Inhalt wiederhergestellt",
        "edit_content" => "Inhalt bearbeitet",
        "report_resolved" => "Meldung erledigt",
        "report_dismissed" => "Meldung verworfen",
        "topic_lock" => "Thema gesperrt",
        "topic_unlock" => "Thema entsperrt",
        "topic_pin" => "Thema angepinnt",
        "topic_unpin" => "Thema gelöst",
        "topic_move" => "Thema verschoben",
        "settings" => "Einstellungen geändert",
        "password_reset" => "Passwort-Reset erzeugt",
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportPage {
    pub items: Vec<ReportView>,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModLogPage {
    pub items: Vec<ModLogEntry>,
    pub total: i64,
}
