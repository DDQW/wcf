//! CMS: admin-defined content types with custom fields stored as JSONB,
//! entries with drafts/scheduling/revisions, static pages, navigation menu
//! and a read-only JSON API.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Map, Value};
use shared::cms::{
    valid_key, CmsTypeInput, CmsTypeView, DisplayValue, EntryEdit, EntryPage, EntryQuery,
    EntryStatus, EntryView, FieldDef, FieldKind, FieldValue, ListLayout, MenuItem, RelationRef,
    RevisionView, ENTRIES_PER_PAGE, MAX_FIELDS,
};
use shared::files::Visibility;
use shared::forum::CategoryAccess;
use shared::{AuthUser, Role, SubjectType};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::users::{opt_user, user_cols};
use crate::{files, markdown, modlog};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TypeRecord {
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
}

const TYPE_COLS: &str = "id, slug, name, name_plural, description, icon, fields, read_access, create_role, \
                         allow_comments, list_layout, is_page_type";

fn type_from(r: &PgRow) -> Result<TypeRecord> {
    let fields: Value = r.try_get("fields")?;
    let read: String = r.try_get("read_access")?;
    let create: String = r.try_get("create_role")?;
    let layout: String = r.try_get("list_layout")?;
    Ok(TypeRecord {
        id: r.try_get("id")?,
        slug: r.try_get("slug")?,
        name: r.try_get("name")?,
        name_plural: r.try_get("name_plural")?,
        description: r.try_get("description")?,
        icon: r.try_get("icon")?,
        fields: serde_json::from_value(fields).unwrap_or_default(),
        read_access: CategoryAccess::parse(&read),
        create_role: Role::parse(&create).unwrap_or(Role::Moderator),
        allow_comments: r.try_get("allow_comments")?,
        list_layout: ListLayout::parse(&layout),
        is_page_type: r.try_get("is_page_type")?,
    })
}

impl TypeRecord {
    pub fn readable(&self, viewer: Option<&AuthUser>) -> bool {
        self.read_access.allows(viewer.map(|v| v.role))
    }

    pub fn can_create(&self, viewer: Option<&AuthUser>) -> bool {
        viewer.is_some_and(|v| v.role.at_least(self.create_role))
    }

    fn view(&self, count: i64, viewer: Option<&AuthUser>) -> CmsTypeView {
        CmsTypeView {
            id: self.id,
            slug: self.slug.clone(),
            name: self.name.clone(),
            name_plural: self.name_plural.clone(),
            description: self.description.clone(),
            icon: self.icon.clone(),
            fields: self.fields.clone(),
            read_access: self.read_access,
            create_role: self.create_role,
            allow_comments: self.allow_comments,
            list_layout: self.list_layout,
            is_page_type: self.is_page_type,
            entry_count: count,
            can_create: self.can_create(viewer),
        }
    }
}

pub async fn type_by_slug(db: &PgPool, slug: &str) -> Result<TypeRecord> {
    let row = sqlx::query(&format!("SELECT {TYPE_COLS} FROM cms_types WHERE slug = $1"))
        .bind(slug)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Inhaltstyp"))?;
    type_from(&row)
}

async fn type_by_id(db: &PgPool, id: Uuid) -> Result<TypeRecord> {
    let row = sqlx::query(&format!("SELECT {TYPE_COLS} FROM cms_types WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Inhaltstyp"))?;
    type_from(&row)
}

pub async fn list_types(db: &PgPool, viewer: Option<&AuthUser>, include_hidden: bool) -> Result<Vec<CmsTypeView>> {
    let rows = sqlx::query(&format!(
        "SELECT {TYPE_COLS}, (SELECT count(*) FROM cms_entries e WHERE e.type_id = t.id AND e.status = 'published' AND e.published_at <= now()) AS n \
         FROM cms_types t ORDER BY is_page_type DESC, name"
    ))
    .fetch_all(db)
    .await?;
    let mut out = Vec::new();
    for r in rows {
        let t = type_from(&r)?;
        if include_hidden || t.readable(viewer) {
            let n: i64 = r.try_get("n")?;
            out.push(t.view(n, viewer));
        }
    }
    Ok(out)
}

pub async fn get_type(db: &PgPool, viewer: Option<&AuthUser>, slug: &str) -> Result<CmsTypeView> {
    let t = type_by_slug(db, slug).await?;
    if !t.readable(viewer) {
        return Err(Error::NotFound("Inhaltstyp"));
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cms_entries WHERE type_id = $1 AND status = 'published' AND published_at <= now()",
    )
    .bind(t.id)
    .fetch_one(db)
    .await?;
    Ok(t.view(n, viewer))
}

fn validate_type(input: &CmsTypeInput) -> Result<()> {
    if !valid_key(&input.slug.replace('-', "_")) || input.slug.len() < 2 {
        return Err(Error::invalid("Das Kürzel darf nur a–z, 0–9 und - enthalten (mind. 2 Zeichen)."));
    }
    if input.name.trim().is_empty() || input.name_plural.trim().is_empty() {
        return Err(Error::invalid("Bitte Name und Mehrzahl angeben."));
    }
    if input.fields.len() > MAX_FIELDS {
        return Err(Error::invalid("Zu viele Felder."));
    }
    let mut keys = HashSet::new();
    for f in &input.fields {
        if !valid_key(&f.key) {
            return Err(Error::invalid(format!(
                "Ungültiger Feldname „{}“: nur a–z, 0–9, _ und mit Buchstaben beginnend.",
                f.key
            )));
        }
        if !keys.insert(f.key.clone()) {
            return Err(Error::invalid(format!("Feldname „{}“ ist doppelt.", f.key)));
        }
        if f.label.trim().is_empty() {
            return Err(Error::invalid(format!("Feld „{}“ braucht eine Beschriftung.", f.key)));
        }
        if f.kind.has_options() && f.options.iter().filter(|o| !o.trim().is_empty()).count() == 0 {
            return Err(Error::invalid(format!("Auswahlfeld „{}“ braucht Optionen.", f.label)));
        }
        if f.kind == FieldKind::Relation && f.relation_type.as_deref().unwrap_or("").is_empty() {
            return Err(Error::invalid(format!("Verknüpfungsfeld „{}“ braucht einen Ziel-Typ.", f.label)));
        }
    }
    Ok(())
}

pub async fn save_type(state: &AppState, actor: &AuthUser, id: Option<Uuid>, mut input: CmsTypeInput) -> Result<Uuid> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    input.slug = input.slug.trim().to_lowercase();
    for f in input.fields.iter_mut() {
        f.options = f.options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
        f.label = f.label.trim().to_string();
    }
    validate_type(&input)?;
    let fields = serde_json::to_value(&input.fields).map_err(Error::internal)?;
    let id = match id {
        Some(id) => {
            let existing = type_by_id(&state.db, id).await?;
            let slug = if existing.is_page_type { existing.slug.clone() } else { input.slug.clone() };
            sqlx::query(
                "UPDATE cms_types SET slug = $2, name = $3, name_plural = $4, description = $5, icon = $6, fields = $7, \
                    read_access = $8, create_role = $9, allow_comments = $10, list_layout = $11, updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .bind(slug)
            .bind(input.name.trim())
            .bind(input.name_plural.trim())
            .bind(input.description.trim())
            .bind(input.icon.trim())
            .bind(fields)
            .bind(input.read_access.as_str())
            .bind(input.create_role.as_str())
            .bind(input.allow_comments)
            .bind(input.list_layout.as_str())
            .execute(&state.db)
            .await
            .map_err(map_unique)?;
            id
        }
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO cms_types (id, slug, name, name_plural, description, icon, fields, read_access, create_role, allow_comments, list_layout) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(id)
            .bind(&input.slug)
            .bind(input.name.trim())
            .bind(input.name_plural.trim())
            .bind(input.description.trim())
            .bind(if input.icon.trim().is_empty() { "layout" } else { input.icon.trim() })
            .bind(fields)
            .bind(input.read_access.as_str())
            .bind(input.create_role.as_str())
            .bind(input.allow_comments)
            .bind(input.list_layout.as_str())
            .execute(&state.db)
            .await
            .map_err(map_unique)?;
            id
        }
    };
    modlog::record(&state.db, Some(actor.id), "settings", None, None, &format!("Inhaltstyp {}", input.name)).await?;
    Ok(id)
}

fn map_unique(e: sqlx::Error) -> Error {
    let e = Error::from(e);
    if e.is_unique_violation(None) {
        Error::invalid("Dieses Kürzel ist bereits vergeben.")
    } else {
        e
    }
}

pub async fn delete_type(state: &AppState, actor: &AuthUser, id: Uuid) -> Result<()> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    let t = type_by_id(&state.db, id).await?;
    if t.is_page_type {
        return Err(Error::invalid("Der Typ „Seite“ kann nicht gelöscht werden."));
    }
    sqlx::query("DELETE FROM cms_types WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    modlog::record(&state.db, Some(actor.id), "delete_content", None, None, &format!("Inhaltstyp {}", t.name)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// Validates and normalises submitted field values. Returns the stored
/// data plus rendered HTML for markdown fields.
async fn validate_data(
    state: &AppState,
    user: &AuthUser,
    ty: &TypeRecord,
    input: &Map<String, Value>,
) -> Result<(Map<String, Value>, Map<String, Value>)> {
    let mut data = Map::new();
    let mut rendered = Map::new();
    for f in &ty.fields {
        let raw = input
            .get(&f.key)
            .cloned()
            .or_else(|| f.default.clone())
            .unwrap_or(Value::Null);
        if is_empty(&raw) {
            if f.required && f.kind != FieldKind::Boolean {
                return Err(Error::invalid(format!("„{}“ ist ein Pflichtfeld.", f.label)));
            }
            if f.kind == FieldKind::Boolean {
                data.insert(f.key.clone(), Value::Bool(false));
            }
            continue;
        }
        let err = |msg: &str| Error::invalid(format!("„{}“: {msg}", f.label));
        let check_len = |s: &str, default_max: usize| -> Result<()> {
            let len = s.chars().count();
            let max = f.max.map(|m| m as usize).unwrap_or(default_max);
            let min = f.min.map(|m| m as usize).unwrap_or(0);
            if len > max {
                return Err(err(&format!("höchstens {max} Zeichen")));
            }
            if len < min {
                return Err(err(&format!("mindestens {min} Zeichen")));
            }
            Ok(())
        };
        let value = match f.kind {
            FieldKind::Text | FieldKind::LongText => {
                let s = raw.as_str().ok_or_else(|| err("Text erwartet"))?.trim().to_string();
                check_len(&s, if f.kind == FieldKind::Text { 500 } else { 20_000 })?;
                Value::String(s)
            }
            FieldKind::Markdown => {
                let s = raw.as_str().ok_or_else(|| err("Text erwartet"))?.to_string();
                check_len(&s, 200_000)?;
                rendered.insert(
                    f.key.clone(),
                    Value::String(markdown::render_mentions(&state.db, &s).await?.html),
                );
                Value::String(s)
            }
            FieldKind::Number | FieldKind::Integer => {
                let n = match &raw {
                    Value::Number(n) => n.as_f64(),
                    Value::String(s) => s.trim().replace(',', ".").parse::<f64>().ok(),
                    _ => None,
                }
                .ok_or_else(|| err("Zahl erwartet"))?;
                if !n.is_finite() {
                    return Err(err("Zahl erwartet"));
                }
                if f.min.is_some_and(|m| n < m) || f.max.is_some_and(|m| n > m) {
                    return Err(err("Wert außerhalb des erlaubten Bereichs"));
                }
                if f.kind == FieldKind::Integer {
                    if n.fract() != 0.0 {
                        return Err(err("Ganzzahl erwartet"));
                    }
                    json!(n as i64)
                } else {
                    json!(n)
                }
            }
            FieldKind::Boolean => Value::Bool(match &raw {
                Value::Bool(b) => *b,
                Value::String(s) => matches!(s.as_str(), "true" | "1" | "on" | "ja"),
                _ => false,
            }),
            FieldKind::Date => {
                let s = raw.as_str().ok_or_else(|| err("Datum erwartet"))?;
                let d = NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| err("Datum im Format JJJJ-MM-TT erwartet"))?;
                Value::String(d.format("%Y-%m-%d").to_string())
            }
            FieldKind::DateTime => {
                let s = raw.as_str().ok_or_else(|| err("Datum erwartet"))?;
                let t = DateTime::parse_from_rfc3339(s.trim()).map_err(|_| err("Datum + Uhrzeit erwartet"))?;
                Value::String(t.with_timezone(&Utc).to_rfc3339())
            }
            FieldKind::Select => {
                let s = raw.as_str().ok_or_else(|| err("Auswahl erwartet"))?;
                if !f.options.iter().any(|o| o == s) {
                    return Err(err("ungültige Auswahl"));
                }
                Value::String(s.to_string())
            }
            FieldKind::MultiSelect => {
                let arr = raw.as_array().ok_or_else(|| err("Liste erwartet"))?;
                let mut out = Vec::new();
                for v in arr {
                    let s = v.as_str().ok_or_else(|| err("ungültige Auswahl"))?;
                    if !f.options.iter().any(|o| o == s) {
                        return Err(err("ungültige Auswahl"));
                    }
                    if !out.contains(&s.to_string()) {
                        out.push(s.to_string());
                    }
                }
                json!(out)
            }
            FieldKind::Url => {
                let s = raw.as_str().ok_or_else(|| err("Link erwartet"))?.trim();
                if !(s.starts_with("https://") || s.starts_with("http://") || s.starts_with('/'))
                    || s.contains(char::is_whitespace)
                    || s.len() > 2000
                {
                    return Err(err("Link muss mit http(s):// oder / beginnen"));
                }
                Value::String(s.to_string())
            }
            FieldKind::Email => {
                let s = raw.as_str().ok_or_else(|| err("E-Mail erwartet"))?.trim();
                shared::util::validate_email(s).map_err(|_| err("ungültige E-Mail-Adresse"))?;
                Value::String(s.to_string())
            }
            FieldKind::Color => {
                let s = raw.as_str().ok_or_else(|| err("Farbe erwartet"))?.trim().to_lowercase();
                if s.len() != 7 || !s.starts_with('#') || !s[1..].chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(err("Farbe im Format #rrggbb erwartet"));
                }
                Value::String(s)
            }
            FieldKind::Image | FieldKind::File => {
                let id = raw
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| err("Datei erwartet"))?;
                let rec = files::get_record(&state.db, id).await?.ok_or_else(|| err("Datei nicht gefunden"))?;
                if rec.owner_id != user.id && !user.is_mod() && !files::can_view(state, &rec, Some(user)).await {
                    return Err(err("Datei nicht verfügbar"));
                }
                if f.kind == FieldKind::Image && !rec.view.mime_type.starts_with("image/") {
                    return Err(err("Bild erwartet"));
                }
                // Make the file as visible as the content type.
                let vis = match ty.read_access {
                    CategoryAccess::Public => Visibility::Public,
                    _ => Visibility::Members,
                };
                if rec.view.visibility != Visibility::Public && rec.view.visibility != vis {
                    sqlx::query("UPDATE files SET visibility = $2 WHERE id = $1")
                        .bind(id)
                        .bind(vis.as_str())
                        .execute(&state.db)
                        .await?;
                }
                Value::String(id.to_string())
            }
            FieldKind::Relation => {
                let target = f.relation_type.clone().unwrap_or_default();
                let ids: Vec<Uuid> = match &raw {
                    Value::Array(a) => a.iter().filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect(),
                    Value::String(s) => Uuid::parse_str(s).ok().into_iter().collect(),
                    _ => Vec::new(),
                };
                if ids.is_empty() {
                    return Err(err("Verknüpfung erwartet"));
                }
                if !f.multiple && ids.len() > 1 {
                    return Err(err("nur eine Verknüpfung erlaubt"));
                }
                let found: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM cms_entries e JOIN cms_types t ON t.id = e.type_id WHERE e.id = ANY($1) AND t.slug = $2",
                )
                .bind(&ids)
                .bind(&target)
                .fetch_one(&state.db)
                .await?;
                if found as usize != ids.len() {
                    return Err(err("verknüpfter Eintrag nicht gefunden"));
                }
                if f.multiple {
                    json!(ids.iter().map(|i| i.to_string()).collect::<Vec<_>>())
                } else {
                    Value::String(ids[0].to_string())
                }
            }
        };
        data.insert(f.key.clone(), value);
    }
    Ok((data, rendered))
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

struct EntryRow {
    id: Uuid,
    slug: String,
    title: String,
    data: Map<String, Value>,
    rendered: Map<String, Value>,
    status: String,
    published_at: Option<DateTime<Utc>>,
    author: Option<shared::UserSummary>,
    author_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn entry_cols() -> String {
    format!(
        "e.id, e.slug, e.title, e.data, e.rendered, e.status, e.published_at, e.author_id AS author_ref, \
         e.created_at, e.updated_at, {}",
        user_cols("u", "author")
    )
}

const ENTRY_FROM: &str = " FROM cms_entries e LEFT JOIN users u ON u.id = e.author_id ";

fn entry_row(r: &PgRow) -> Result<EntryRow> {
    let data: Value = r.try_get("data")?;
    let rendered: Value = r.try_get("rendered")?;
    Ok(EntryRow {
        id: r.try_get("id")?,
        slug: r.try_get("slug")?,
        title: r.try_get("title")?,
        data: data.as_object().cloned().unwrap_or_default(),
        rendered: rendered.as_object().cloned().unwrap_or_default(),
        status: r.try_get("status")?,
        published_at: r.try_get("published_at")?,
        author: opt_user(r, "author")?,
        author_id: r.try_get("author_ref")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

fn status_of(status: &str, published_at: Option<DateTime<Utc>>) -> EntryStatus {
    match (status, published_at) {
        ("published", Some(t)) if t > Utc::now() => EntryStatus::Scheduled,
        ("published", _) => EntryStatus::Published,
        ("archived", _) => EntryStatus::Archived,
        _ => EntryStatus::Draft,
    }
}

fn can_edit(ty: &TypeRecord, viewer: Option<&AuthUser>, author: Option<Uuid>) -> bool {
    viewer.is_some_and(|v| v.is_mod() || (author == Some(v.id) && ty.can_create(Some(v))))
}

/// Turns raw rows into display views, resolving files and relations in bulk.
async fn build_views(state: &AppState, ty: &TypeRecord, viewer: Option<&AuthUser>, rows: Vec<EntryRow>) -> Result<Vec<EntryView>> {
    let mut file_ids = Vec::new();
    let mut rel_ids = Vec::new();
    for r in &rows {
        for f in &ty.fields {
            let Some(v) = r.data.get(&f.key) else { continue };
            match f.kind {
                FieldKind::Image | FieldKind::File => {
                    if let Some(id) = v.as_str().and_then(|s| Uuid::parse_str(s).ok()) {
                        file_ids.push(id);
                    }
                }
                FieldKind::Relation => match v {
                    Value::Array(a) => rel_ids.extend(a.iter().filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))),
                    Value::String(s) => rel_ids.extend(Uuid::parse_str(s).ok()),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    let files_map: HashMap<Uuid, shared::files::FileView> = files::get_records(&state.db, &file_ids)
        .await?
        .into_iter()
        .filter(|r| r.view.deleted_at.is_none())
        .map(|r| (r.view.id, r.view))
        .collect();
    let rel_rows: Vec<(Uuid, String, String, String)> = if rel_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT e.id, e.title, e.slug, t.slug FROM cms_entries e JOIN cms_types t ON t.id = e.type_id \
             WHERE e.id = ANY($1) AND e.status = 'published' AND e.published_at <= now()",
        )
        .bind(&rel_ids)
        .fetch_all(&state.db)
        .await?
    };
    let rel_map: HashMap<Uuid, RelationRef> = rel_rows
        .into_iter()
        .map(|(id, title, slug, tslug)| (id, RelationRef { id, title, url: format!("/c/{tslug}/{slug}") }))
        .collect();

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let mut values = Vec::new();
        let mut cover = None;
        for f in &ty.fields {
            let v = r.data.get(&f.key).cloned().unwrap_or(Value::Null);
            let display = match f.kind {
                _ if is_empty(&v) && f.kind != FieldKind::Boolean => DisplayValue::Empty,
                FieldKind::Email => {
                    let s = v.as_str().unwrap_or_default().to_string();
                    DisplayValue::Link { url: format!("mailto:{s}"), label: s }
                }
                FieldKind::Text | FieldKind::LongText => DisplayValue::Text(v.as_str().unwrap_or_default().to_string()),
                FieldKind::Markdown => DisplayValue::Html(
                    r.rendered.get(&f.key).and_then(|h| h.as_str()).unwrap_or_default().to_string(),
                ),
                FieldKind::Number => DisplayValue::Text(
                    v.as_f64().map(|n| format!("{n}").replace('.', ",")).unwrap_or_default(),
                ),
                FieldKind::Integer => DisplayValue::Text(v.as_i64().map(|n| n.to_string()).unwrap_or_default()),
                FieldKind::Boolean => DisplayValue::Bool(v.as_bool().unwrap_or(false)),
                FieldKind::Date => DisplayValue::Text(
                    v.as_str()
                        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                        .map(|d| d.format("%d.%m.%Y").to_string())
                        .unwrap_or_default(),
                ),
                FieldKind::DateTime => DisplayValue::Text(
                    v.as_str()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.with_timezone(&Utc).format("%d.%m.%Y %H:%M UTC").to_string())
                        .unwrap_or_default(),
                ),
                FieldKind::Select => DisplayValue::Tags(vec![v.as_str().unwrap_or_default().to_string()]),
                FieldKind::MultiSelect => DisplayValue::Tags(
                    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                ),
                FieldKind::Url => {
                    let s = v.as_str().unwrap_or_default().to_string();
                    DisplayValue::Link { label: s.trim_start_matches("https://").trim_start_matches("http://").to_string(), url: s }
                }
                FieldKind::Color => DisplayValue::Color(v.as_str().unwrap_or_default().to_string()),
                FieldKind::Image => match v.as_str().and_then(|s| Uuid::parse_str(s).ok()).and_then(|id| files_map.get(&id)) {
                    Some(fv) => {
                        if cover.is_none() {
                            cover = Some(fv.preview_url());
                        }
                        DisplayValue::Image {
                            url: fv.preview_url(),
                            thumb: fv.thumb_url().unwrap_or_else(|| fv.url()),
                        }
                    }
                    None => DisplayValue::Empty,
                },
                FieldKind::File => match v.as_str().and_then(|s| Uuid::parse_str(s).ok()).and_then(|id| files_map.get(&id)) {
                    Some(fv) => DisplayValue::File { url: fv.download_url(), name: fv.name.clone() },
                    None => DisplayValue::Empty,
                },
                FieldKind::Relation => {
                    let ids: Vec<Uuid> = match &v {
                        Value::Array(a) => a.iter().filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect(),
                        Value::String(s) => Uuid::parse_str(s).ok().into_iter().collect(),
                        _ => Vec::new(),
                    };
                    DisplayValue::Relations(ids.iter().filter_map(|id| rel_map.get(id).cloned()).collect())
                }
            };
            values.push(FieldValue {
                key: f.key.clone(),
                label: f.label.clone(),
                kind: f.kind,
                show_in_list: f.show_in_list,
                value: display,
            });
        }
        out.push(EntryView {
            id: r.id,
            type_slug: ty.slug.clone(),
            type_name: ty.name.clone(),
            slug: r.slug,
            title: r.title,
            status: status_of(&r.status, r.published_at),
            published_at: r.published_at,
            author: r.author,
            created_at: r.created_at,
            updated_at: r.updated_at,
            values,
            cover_url: cover,
            can_edit: can_edit(ty, viewer, r.author_id),
            allow_comments: ty.allow_comments,
        });
    }
    Ok(out)
}

pub async fn list_entries(state: &AppState, viewer: Option<&AuthUser>, type_slug: &str, q: EntryQuery) -> Result<EntryPage> {
    let ty = type_by_slug(&state.db, type_slug).await?;
    if !ty.readable(viewer) {
        return Err(Error::NotFound("Inhaltstyp"));
    }
    let editor = viewer.is_some_and(|v| v.is_mod() || ty.can_create(Some(v)));
    let all = q.all_statuses && editor;

    let build = |select: &str| -> QueryBuilder<'static, Postgres> {
        let mut qb = QueryBuilder::new(format!("SELECT {select}{ENTRY_FROM} WHERE e.type_id = "));
        qb.push_bind(ty.id);
        if all {
            if !viewer.is_some_and(|v| v.is_mod()) {
                // Non-staff editors see published entries plus their own.
                qb.push(" AND ((e.status = 'published' AND e.published_at <= now()) OR e.author_id = ");
                qb.push_bind(viewer.map(|v| v.id));
                qb.push(")");
            }
        } else {
            qb.push(" AND e.status = 'published' AND e.published_at <= now()");
        }
        if let Some(s) = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            qb.push(" AND (e.search @@ websearch_to_tsquery('german', ");
            qb.push_bind(s.to_string());
            qb.push(") OR position(lower(");
            qb.push_bind(s.to_string());
            qb.push(") in lower(e.title)) > 0)");
        }
        let mut containment = Map::new();
        for (key, value) in &q.filters {
            let Some(f) = ty.fields.iter().find(|f| &f.key == key && f.kind.filterable()) else { continue };
            let v = match f.kind {
                FieldKind::Boolean => Value::Bool(value == "true"),
                FieldKind::MultiSelect => json!([value]),
                _ => Value::String(value.clone()),
            };
            containment.insert(key.clone(), v);
        }
        if !containment.is_empty() {
            qb.push(" AND e.data @> ");
            qb.push_bind(Value::Object(containment));
        }
        qb
    };

    let mut count_qb = build("count(*) AS n");
    let total: i64 = count_qb.build().fetch_one(&state.db).await?.try_get("n")?;

    let mut qb = build(&entry_cols());
    let dir = if q.descending { "DESC" } else { "ASC" };
    match q.sort.as_deref() {
        Some("title") => {
            qb.push(format!(" ORDER BY lower(e.title) {dir}"));
        }
        Some(key) if ty.fields.iter().any(|f| f.key == key) => {
            let f = ty.fields.iter().find(|f| f.key == key).expect("checked");
            let expr = match f.kind {
                FieldKind::Number | FieldKind::Integer => "(e.data->>",
                _ => "lower(e.data->>",
            };
            qb.push(format!(" ORDER BY {expr}"));
            qb.push_bind(key.to_string());
            if matches!(f.kind, FieldKind::Number | FieldKind::Integer) {
                qb.push(format!(")::numeric {dir} NULLS LAST"));
            } else {
                qb.push(format!(") {dir} NULLS LAST"));
            }
        }
        _ => {
            qb.push(format!(" ORDER BY coalesce(e.published_at, e.updated_at) {dir}"));
        }
    }
    qb.push(" LIMIT ");
    qb.push_bind(ENTRIES_PER_PAGE);
    qb.push(" OFFSET ");
    qb.push_bind(q.page.max(0) * ENTRIES_PER_PAGE);
    let rows = qb.build().fetch_all(&state.db).await?;
    let rows: Vec<EntryRow> = rows.iter().map(entry_row).collect::<Result<_>>()?;
    let items = build_views(state, &ty, viewer, rows).await?;
    Ok(EntryPage { items, total })
}

pub async fn get_entry(state: &AppState, viewer: Option<&AuthUser>, type_slug: &str, slug: &str) -> Result<EntryView> {
    let ty = type_by_slug(&state.db, type_slug).await?;
    if !ty.readable(viewer) {
        return Err(Error::NotFound("Eintrag"));
    }
    let row = sqlx::query(&format!("SELECT {}{ENTRY_FROM} WHERE e.type_id = $1 AND e.slug = $2", entry_cols()))
        .bind(ty.id)
        .bind(slug)
        .fetch_optional(&state.db)
        .await?
        .ok_or(Error::NotFound("Eintrag"))?;
    let r = entry_row(&row)?;
    let visible = status_of(&r.status, r.published_at) == EntryStatus::Published
        || viewer.is_some_and(|v| v.is_mod() || Some(v.id) == r.author_id);
    if !visible {
        return Err(Error::NotFound("Eintrag"));
    }
    Ok(build_views(state, &ty, viewer, vec![r]).await?.remove(0))
}

/// Static page `/p/<slug>` from the built-in page type.
pub async fn page(state: &AppState, viewer: Option<&AuthUser>, slug: &str) -> Result<EntryView> {
    let slug_type: String = sqlx::query_scalar("SELECT slug FROM cms_types WHERE is_page_type LIMIT 1")
        .fetch_one(&state.db)
        .await?;
    get_entry(state, viewer, &slug_type, slug).await
}

pub async fn entry_for_edit(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<EntryEdit> {
    let row = sqlx::query(
        "SELECT e.id, e.slug, e.title, e.data, e.status, e.published_at, e.author_id, t.slug AS type_slug \
         FROM cms_entries e JOIN cms_types t ON t.id = e.type_id WHERE e.id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Eintrag"))?;
    let type_slug: String = row.try_get("type_slug")?;
    let ty = type_by_slug(db, &type_slug).await?;
    let author: Option<Uuid> = row.try_get("author_id")?;
    if !can_edit(&ty, Some(user), author) {
        return Err(Error::Forbidden);
    }
    let data: Value = row.try_get("data")?;
    let status: String = row.try_get("status")?;
    let published_at: Option<DateTime<Utc>> = row.try_get("published_at")?;
    Ok(EntryEdit {
        id: Some(id),
        type_slug,
        title: row.try_get("title")?,
        slug: row.try_get("slug")?,
        data: data.as_object().cloned().unwrap_or_default(),
        status: status_of(&status, published_at),
        publish_at: published_at,
    })
}

async fn unique_slug(db: &PgPool, type_id: Uuid, wanted: &str, exclude: Option<Uuid>) -> Result<String> {
    let base = {
        let b = shared::util::slug_base(wanted);
        if b.is_empty() { "eintrag".to_string() } else { b }
    };
    let mut candidate = base.clone();
    for i in 2..500 {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM cms_entries WHERE type_id = $1 AND slug = $2 AND ($3::uuid IS NULL OR id <> $3))",
        )
        .bind(type_id)
        .bind(&candidate)
        .bind(exclude)
        .fetch_one(db)
        .await?;
        if !taken {
            return Ok(candidate);
        }
        candidate = format!("{base}-{i}");
    }
    Ok(format!("{base}-{}", &Uuid::now_v7().simple().to_string()[..6]))
}

pub async fn save_entry(state: &AppState, user: &AuthUser, input: EntryEdit) -> Result<String> {
    if !state.settings().modules.cms {
        return Err(Error::invalid("Das CMS ist deaktiviert."));
    }
    let ty = type_by_slug(&state.db, &input.type_slug).await?;
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 300 {
        return Err(Error::invalid("Der Titel muss 1–300 Zeichen lang sein."));
    }
    let existing: Option<(Option<Uuid>, String, Value, String, Uuid)> = match input.id {
        Some(id) => Some(
            sqlx::query_as("SELECT author_id, title, data, status, type_id FROM cms_entries WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.db)
                .await?
                .ok_or(Error::NotFound("Eintrag"))?,
        ),
        None => None,
    };
    match &existing {
        Some((author, _, _, _, type_id)) => {
            if *type_id != ty.id || !can_edit(&ty, Some(user), *author) {
                return Err(Error::Forbidden);
            }
        }
        None => {
            if !ty.can_create(Some(user)) {
                return Err(Error::Forbidden);
            }
        }
    }
    let (data, rendered) = validate_data(state, user, &ty, &input.data).await?;
    let slug = unique_slug(
        &state.db,
        ty.id,
        if input.slug.trim().is_empty() { title } else { input.slug.trim() },
        input.id,
    )
    .await?;
    let (status, published_at) = match input.status {
        EntryStatus::Draft => ("draft", input.publish_at),
        EntryStatus::Archived => ("archived", input.publish_at),
        EntryStatus::Published => ("published", Some(input.publish_at.filter(|t| *t <= Utc::now()).unwrap_or_else(Utc::now))),
        EntryStatus::Scheduled => (
            "published",
            Some(input.publish_at.ok_or_else(|| Error::invalid("Bitte einen Veröffentlichungszeitpunkt wählen."))?),
        ),
    };

    let id = match (input.id, existing) {
        (Some(id), Some((author, old_title, old_data, old_status, _))) => {
            sqlx::query("INSERT INTO cms_revisions (id, entry_id, title, data, status, editor_id) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(Uuid::now_v7())
                .bind(id)
                .bind(&old_title)
                .bind(&old_data)
                .bind(&old_status)
                .bind(user.id)
                .execute(&state.db)
                .await?;
            sqlx::query(
                "UPDATE cms_entries SET title = $2, slug = $3, data = $4, rendered = $5, status = $6, published_at = $7, \
                    updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .bind(title)
            .bind(&slug)
            .bind(Value::Object(data))
            .bind(Value::Object(rendered))
            .bind(status)
            .bind(published_at)
            .execute(&state.db)
            .await?;
            if author != Some(user.id) {
                modlog::record(&state.db, Some(user.id), "edit_content", Some((SubjectType::CmsEntry, id)), author, title).await?;
            }
            id
        }
        _ => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO cms_entries (id, type_id, slug, title, data, rendered, status, published_at, author_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(id)
            .bind(ty.id)
            .bind(&slug)
            .bind(title)
            .bind(Value::Object(data))
            .bind(Value::Object(rendered))
            .bind(status)
            .bind(published_at)
            .bind(user.id)
            .execute(&state.db)
            .await?;
            id
        }
    };
    let _ = id;
    Ok(if ty.is_page_type { format!("/p/{slug}") } else { format!("/c/{}/{slug}", ty.slug) })
}

pub async fn delete_entry(state: &AppState, user: &AuthUser, id: Uuid) -> Result<String> {
    let row: Option<(Option<Uuid>, String, String)> = sqlx::query_as(
        "SELECT e.author_id, e.title, t.slug FROM cms_entries e JOIN cms_types t ON t.id = e.type_id WHERE e.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (author, title, type_slug) = row.ok_or(Error::NotFound("Eintrag"))?;
    let ty = type_by_slug(&state.db, &type_slug).await?;
    if !can_edit(&ty, Some(user), author) {
        return Err(Error::Forbidden);
    }
    sqlx::query("DELETE FROM cms_entries WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM comments WHERE subject_type = 'cms_entry' AND subject_id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if author != Some(user.id) {
        modlog::record(&state.db, Some(user.id), "delete_content", Some((SubjectType::CmsEntry, id)), author, &title).await?;
    }
    Ok(type_slug)
}

pub async fn revisions(db: &PgPool, user: &AuthUser, entry_id: Uuid) -> Result<Vec<RevisionView>> {
    entry_for_edit(db, user, entry_id).await?;
    let rows = sqlx::query(&format!(
        "SELECT r.id, r.title, r.status, r.created_at, {} FROM cms_revisions r LEFT JOIN users u ON u.id = r.editor_id \
         WHERE r.entry_id = $1 ORDER BY r.created_at DESC LIMIT 100",
        user_cols("u", "editor")
    ))
    .bind(entry_id)
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(RevisionView {
                id: r.try_get("id")?,
                title: r.try_get("title")?,
                status: r.try_get("status")?,
                editor: opt_user(r, "editor")?,
                created_at: r.try_get("created_at")?,
            })
        })
        .collect()
}

/// Restores a revision by saving its data as a new version (validated
/// against the current field definitions).
pub async fn restore_revision(state: &AppState, user: &AuthUser, revision_id: Uuid) -> Result<String> {
    let row: Option<(Uuid, String, Value)> =
        sqlx::query_as("SELECT entry_id, title, data FROM cms_revisions WHERE id = $1")
            .bind(revision_id)
            .fetch_optional(&state.db)
            .await?;
    let (entry_id, title, data) = row.ok_or(Error::NotFound("Version"))?;
    let mut edit = entry_for_edit(&state.db, user, entry_id).await?;
    edit.title = title;
    edit.data = data.as_object().cloned().unwrap_or_default();
    save_entry(state, user, edit).await
}

/// Candidates for relation fields (title search within a type).
pub async fn relation_options(state: &AppState, viewer: Option<&AuthUser>, type_slug: &str, q: &str) -> Result<Vec<RelationRef>> {
    let ty = type_by_slug(&state.db, type_slug).await?;
    if !ty.readable(viewer) {
        return Err(Error::NotFound("Inhaltstyp"));
    }
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, title, slug FROM cms_entries WHERE type_id = $1 AND status = 'published' \
           AND ($2 = '' OR position(lower($2) in lower(title)) > 0) ORDER BY lower(title) LIMIT 30",
    )
    .bind(ty.id)
    .bind(q.trim())
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, title, slug)| RelationRef { id, title, url: format!("/c/{}/{slug}", ty.slug) })
        .collect())
}

/// Titles of given entries (to show already selected relations in forms).
pub async fn relation_titles(db: &PgPool, ids: &[Uuid]) -> Result<Vec<RelationRef>> {
    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT e.id, e.title, e.slug, t.slug FROM cms_entries e JOIN cms_types t ON t.id = e.type_id WHERE e.id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, title, slug, ts)| RelationRef { id, title, url: format!("/c/{ts}/{slug}") })
        .collect())
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

pub async fn menu(db: &PgPool) -> Result<Vec<MenuItem>> {
    let rows: Vec<(Uuid, String, String, i32)> =
        sqlx::query_as("SELECT id, label, url, position FROM cms_menu ORDER BY position, label")
            .fetch_all(db)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, label, url, position)| MenuItem { id, label, url, position })
        .collect())
}

pub async fn save_menu(state: &AppState, actor: &AuthUser, items: Vec<MenuItem>) -> Result<()> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    if items.len() > 30 {
        return Err(Error::invalid("Höchstens 30 Menüpunkte."));
    }
    for i in &items {
        if i.label.trim().is_empty() || i.label.chars().count() > 40 {
            return Err(Error::invalid("Jeder Menüpunkt braucht eine Beschriftung (max. 40 Zeichen)."));
        }
        let url = i.url.trim();
        if !(url.starts_with('/') || url.starts_with("https://") || url.starts_with("http://")) {
            return Err(Error::invalid("Links müssen mit / oder http(s):// beginnen."));
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM cms_menu").execute(&mut *tx).await?;
    for (pos, i) in items.iter().enumerate() {
        sqlx::query("INSERT INTO cms_menu (id, label, url, position) VALUES ($1, $2, $3, $4)")
            .bind(Uuid::now_v7())
            .bind(i.label.trim())
            .bind(i.url.trim())
            .bind(pos as i32)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Subjects (comments, reports, reactions)
// ---------------------------------------------------------------------------

pub async fn subject_info(state: &AppState, viewer: Option<&AuthUser>, id: Uuid) -> Result<Option<crate::subjects::SubjectInfo>> {
    let row: Option<(String, String, String, Option<DateTime<Utc>>, Option<Uuid>, String, bool)> = sqlx::query_as(
        "SELECT e.title, e.slug, e.status, e.published_at, e.author_id, t.slug, t.is_page_type \
         FROM cms_entries e JOIN cms_types t ON t.id = e.type_id WHERE e.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let Some((title, slug, status, published_at, author, type_slug, is_page)) = row else { return Ok(None) };
    let ty = type_by_slug(&state.db, &type_slug).await?;
    let visible = ty.readable(viewer)
        && (status_of(&status, published_at) == EntryStatus::Published || viewer.is_some_and(|v| v.is_mod() || Some(v.id) == author));
    if !visible {
        return Ok(None);
    }
    Ok(Some(crate::subjects::SubjectInfo {
        author_id: author,
        preview: title.clone(),
        title,
        link: Some(if is_page { format!("/p/{slug}") } else { format!("/c/{type_slug}/{slug}") }),
    }))
}

pub async fn comments_allowed(db: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT t.allow_comments FROM cms_entries e JOIN cms_types t ON t.id = e.type_id WHERE e.id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .unwrap_or(false))
}

pub async fn owner_of(db: &PgPool, id: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Option<Uuid>>("SELECT author_id FROM cms_entries WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .flatten())
}

pub async fn total_entries(db: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM cms_entries").fetch_one(db).await?)
}

// ---------------------------------------------------------------------------
// JSON API
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct ApiParams {
    pub search: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub page: Option<i64>,
    #[serde(flatten)]
    pub rest: HashMap<String, String>,
}

pub async fn api_list(
    axum::extract::State(state): axum::extract::State<AppState>,
    session: tower_sessions::Session,
    axum::extract::Path(type_slug): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<ApiParams>,
) -> std::result::Result<axum::Json<Value>, Error> {
    if !state.settings().modules.cms {
        return Err(Error::NotFound("API"));
    }
    let viewer = crate::auth::user_from_session(&state.db, &session).await.ok().flatten();
    let filters = params
        .rest
        .iter()
        .filter_map(|(k, v)| k.strip_prefix("f.").map(|key| (key.to_string(), v.clone())))
        .collect();
    let page = list_entries(
        &state,
        viewer.as_ref(),
        &type_slug,
        EntryQuery {
            search: params.search,
            filters,
            sort: params.sort,
            descending: params.order.as_deref() != Some("asc"),
            page: params.page.unwrap_or(0),
            all_statuses: false,
        },
    )
    .await?;
    let ids: Vec<Uuid> = page.items.iter().map(|e| e.id).collect();
    let raw: HashMap<Uuid, Value> = sqlx::query_as::<_, (Uuid, Value)>("SELECT id, data FROM cms_entries WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .collect();
    let items: Vec<Value> = page
        .items
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "slug": e.slug,
                "title": e.title,
                "url": state.config.url(&e.url()),
                "published_at": e.published_at,
                "updated_at": e.updated_at,
                "author": e.author.as_ref().map(|a| a.username.clone()),
                "data": raw.get(&e.id).cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(axum::Json(json!({ "total": page.total, "per_page": ENTRIES_PER_PAGE, "items": items })))
}

pub async fn api_get(
    axum::extract::State(state): axum::extract::State<AppState>,
    session: tower_sessions::Session,
    axum::extract::Path((type_slug, slug)): axum::extract::Path<(String, String)>,
) -> std::result::Result<axum::Json<Value>, Error> {
    if !state.settings().modules.cms {
        return Err(Error::NotFound("API"));
    }
    let viewer = crate::auth::user_from_session(&state.db, &session).await.ok().flatten();
    let e = get_entry(&state, viewer.as_ref(), &type_slug, &slug).await?;
    if e.status != EntryStatus::Published {
        return Err(Error::NotFound("Eintrag"));
    }
    let (data, rendered): (Value, Value) = sqlx::query_as("SELECT data, rendered FROM cms_entries WHERE id = $1")
        .bind(e.id)
        .fetch_one(&state.db)
        .await?;
    Ok(axum::Json(json!({
        "id": e.id,
        "slug": e.slug,
        "title": e.title,
        "type": e.type_slug,
        "url": state.config.url(&e.url()),
        "published_at": e.published_at,
        "updated_at": e.updated_at,
        "author": e.author.as_ref().map(|a| a.username.clone()),
        "data": data,
        "html": rendered,
    })))
}
