use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::forum::CategoryAccess;
use crate::user::{Role, UserSummary};

pub const ENTRIES_PER_PAGE: i64 = 24;
pub const MAX_FIELDS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    Text,
    LongText,
    Markdown,
    Number,
    Integer,
    Boolean,
    Date,
    DateTime,
    Select,
    MultiSelect,
    Url,
    Email,
    Color,
    Image,
    File,
    Relation,
}

impl FieldKind {
    pub const ALL: [FieldKind; 16] = [
        FieldKind::Text,
        FieldKind::LongText,
        FieldKind::Markdown,
        FieldKind::Number,
        FieldKind::Integer,
        FieldKind::Boolean,
        FieldKind::Date,
        FieldKind::DateTime,
        FieldKind::Select,
        FieldKind::MultiSelect,
        FieldKind::Url,
        FieldKind::Email,
        FieldKind::Color,
        FieldKind::Image,
        FieldKind::File,
        FieldKind::Relation,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::LongText => "long_text",
            FieldKind::Markdown => "markdown",
            FieldKind::Number => "number",
            FieldKind::Integer => "integer",
            FieldKind::Boolean => "boolean",
            FieldKind::Date => "date",
            FieldKind::DateTime => "date_time",
            FieldKind::Select => "select",
            FieldKind::MultiSelect => "multi_select",
            FieldKind::Url => "url",
            FieldKind::Email => "email",
            FieldKind::Color => "color",
            FieldKind::Image => "image",
            FieldKind::File => "file",
            FieldKind::Relation => "relation",
        }
    }

    pub fn parse(s: &str) -> FieldKind {
        FieldKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .unwrap_or_default()
    }

    pub fn label(&self) -> &'static str {
        match self {
            FieldKind::Text => "Text (einzeilig)",
            FieldKind::LongText => "Text (mehrzeilig)",
            FieldKind::Markdown => "Formatierter Text (Markdown)",
            FieldKind::Number => "Zahl",
            FieldKind::Integer => "Ganzzahl",
            FieldKind::Boolean => "Ja/Nein",
            FieldKind::Date => "Datum",
            FieldKind::DateTime => "Datum + Uhrzeit",
            FieldKind::Select => "Auswahl",
            FieldKind::MultiSelect => "Mehrfachauswahl",
            FieldKind::Url => "Link (URL)",
            FieldKind::Email => "E-Mail",
            FieldKind::Color => "Farbe",
            FieldKind::Image => "Bild",
            FieldKind::File => "Datei",
            FieldKind::Relation => "Verknüpfung zu Einträgen",
        }
    }

    pub fn has_options(&self) -> bool {
        matches!(self, FieldKind::Select | FieldKind::MultiSelect)
    }

    pub fn has_range(&self) -> bool {
        matches!(self, FieldKind::Number | FieldKind::Integer | FieldKind::Text | FieldKind::LongText)
    }

    /// Fields usable as list filters on public pages.
    pub fn filterable(&self) -> bool {
        matches!(self, FieldKind::Select | FieldKind::MultiSelect | FieldKind::Boolean)
    }
}

/// Definition of one custom field of a content type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldDef {
    /// Machine name: lowercase letters, digits, underscores.
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
    pub help: String,
    /// Choices for select fields.
    pub options: Vec<String>,
    /// Min/max value (numbers) or length (text).
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Target type slug for relations.
    pub relation_type: Option<String>,
    /// Several related entries allowed.
    pub multiple: bool,
    /// Show the value in list views.
    pub show_in_list: bool,
    /// Default value as JSON.
    pub default: Option<serde_json::Value>,
}

impl Default for FieldDef {
    fn default() -> Self {
        Self {
            key: String::new(),
            label: String::new(),
            kind: FieldKind::Text,
            required: false,
            help: String::new(),
            options: Vec::new(),
            min: None,
            max: None,
            relation_type: None,
            multiple: false,
            show_in_list: false,
            default: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListLayout {
    #[default]
    Cards,
    List,
    Table,
}

impl ListLayout {
    pub fn as_str(&self) -> &'static str {
        match self {
            ListLayout::Cards => "cards",
            ListLayout::List => "list",
            ListLayout::Table => "table",
        }
    }

    pub fn parse(s: &str) -> ListLayout {
        match s {
            "list" => ListLayout::List,
            "table" => ListLayout::Table,
            _ => ListLayout::Cards,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ListLayout::Cards => "Karten",
            ListLayout::List => "Liste",
            ListLayout::Table => "Tabelle",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmsTypeView {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub name_plural: String,
    pub description: String,
    pub icon: String,
    pub fields: Vec<FieldDef>,
    pub read_access: CategoryAccess,
    pub create_role: Role,
    pub allow_comments: bool,
    pub list_layout: ListLayout,
    pub is_page_type: bool,
    pub entry_count: i64,
    pub can_create: bool,
}

/// Editable parts of a content type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmsTypeInput {
    pub slug: String,
    pub name: String,
    pub name_plural: String,
    pub description: String,
    pub icon: String,
    pub fields: Vec<FieldDef>,
    pub read_access: CategoryAccess,
    pub create_role: Role,
    pub allow_comments: bool,
    pub list_layout: ListLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    #[default]
    Draft,
    Published,
    Scheduled,
    Archived,
}

impl EntryStatus {
    pub fn label(&self) -> &'static str {
        match self {
            EntryStatus::Draft => "Entwurf",
            EntryStatus::Published => "Veröffentlicht",
            EntryStatus::Scheduled => "Geplant",
            EntryStatus::Archived => "Archiviert",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EntryStatus::Draft => "draft",
            EntryStatus::Published => "published",
            EntryStatus::Scheduled => "published",
            EntryStatus::Archived => "archived",
        }
    }
}

/// A field value prepared for display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum DisplayValue {
    Empty,
    Text(String),
    Html(String),
    Bool(bool),
    Link { url: String, label: String },
    Color(String),
    Image { url: String, thumb: String },
    File { url: String, name: String },
    Tags(Vec<String>),
    Relations(Vec<RelationRef>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationRef {
    pub id: Uuid,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldValue {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub show_in_list: bool,
    pub value: DisplayValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryView {
    pub id: Uuid,
    pub type_slug: String,
    pub type_name: String,
    pub slug: String,
    pub title: String,
    pub status: EntryStatus,
    pub published_at: Option<DateTime<Utc>>,
    pub author: Option<UserSummary>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub values: Vec<FieldValue>,
    /// First image field (cards).
    pub cover_url: Option<String>,
    pub can_edit: bool,
    pub allow_comments: bool,
}

impl EntryView {
    pub fn url(&self) -> String {
        format!("/c/{}/{}", self.type_slug, self.slug)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryPage {
    pub items: Vec<EntryView>,
    pub total: i64,
}

/// Raw entry for the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryEdit {
    pub id: Option<Uuid>,
    pub type_slug: String,
    pub title: String,
    pub slug: String,
    /// Field values keyed by field key (JSON).
    pub data: serde_json::Map<String, serde_json::Value>,
    pub status: EntryStatus,
    pub publish_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionView {
    pub id: Uuid,
    pub title: String,
    pub status: String,
    pub editor: Option<UserSummary>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryQuery {
    pub search: Option<String>,
    /// field key → required value (select/boolean filters).
    pub filters: Vec<(String, String)>,
    /// Field key or "title" / "published".
    pub sort: Option<String>,
    pub descending: bool,
    pub page: i64,
    /// Include drafts/archived (editors only).
    pub all_statuses: bool,
}

impl Default for EntryQuery {
    fn default() -> Self {
        Self {
            search: None,
            filters: Vec::new(),
            sort: None,
            descending: true,
            page: 0,
            all_statuses: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    pub id: Uuid,
    pub label: String,
    pub url: String,
    pub position: i32,
}

/// Valid machine names: `a-z`, `0-9`, `_`, starting with a letter.
pub fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && key.len() <= 40
        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Suggests a key from a label ("Preis (€)" → "preis").
pub fn key_from_label(label: &str) -> String {
    let base = crate::util::slug_base(label).replace('-', "_");
    let trimmed: String = base.chars().take(40).collect();
    if trimmed.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        trimmed
    } else {
        format!("feld_{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert!(valid_key("preis"));
        assert!(valid_key("start_datum2"));
        assert!(!valid_key("2abc"));
        assert!(!valid_key("Preis"));
        assert!(!valid_key("mit-strich"));
        assert_eq!(key_from_label("Preis (€)"), "preis");
        assert_eq!(key_from_label("Größe in cm"), "groesse_in_cm");
        assert_eq!(key_from_label("123"), "feld_123");
    }
}
