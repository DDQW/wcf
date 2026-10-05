//! Filebase: uploads, folders, trash, visibility, share links, quotas.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use shared::files::{
    DriveListing, DriveSort, FilePurpose, FileView, FolderRef, FolderView, ShareInfo, ShareView,
    StorageUsage, Visibility,
};
use shared::{AuthUser, SubjectType};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::media;
use crate::state::AppState;
use crate::storage::TempUpload;
use crate::users::{user_cols, user_from};

pub const TRASH_DAYS: i64 = 30;
const MAX_NAME: usize = 200;

/// Internal file record (includes storage details never sent to clients).
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub view: FileView,
    pub owner_id: Uuid,
    pub storage_key: String,
    pub context_id: Option<Uuid>,
}

pub fn file_cols() -> String {
    format!(
        "f.id, f.name, f.mime_type, f.size_bytes, f.width, f.height, f.duration_secs, f.has_thumb, \
         f.visibility, f.purpose, f.folder_id, f.description, f.download_count, f.created_at, \
         f.updated_at, f.deleted_at, f.storage_key, f.owner_id, f.context_id, {}",
        user_cols("u", "owner")
    )
}

pub const FILE_JOIN: &str = " FROM files f JOIN users u ON u.id = f.owner_id ";

pub fn record_from(r: &PgRow) -> Result<FileRecord, sqlx::Error> {
    let visibility: String = r.try_get("visibility")?;
    let purpose: String = r.try_get("purpose")?;
    Ok(FileRecord {
        view: FileView {
            id: r.try_get("id")?,
            name: r.try_get("name")?,
            mime_type: r.try_get("mime_type")?,
            size_bytes: r.try_get("size_bytes")?,
            width: r.try_get("width")?,
            height: r.try_get("height")?,
            duration_secs: r.try_get("duration_secs")?,
            has_thumb: r.try_get("has_thumb")?,
            visibility: Visibility::parse(&visibility),
            purpose: FilePurpose::parse(&purpose),
            folder_id: r.try_get("folder_id")?,
            description: r.try_get("description")?,
            download_count: r.try_get("download_count")?,
            owner: user_from(r, "owner")?,
            created_at: r.try_get("created_at")?,
            updated_at: r.try_get("updated_at")?,
            deleted_at: r.try_get("deleted_at")?,
        },
        owner_id: r.try_get("owner_id")?,
        storage_key: r.try_get("storage_key")?,
        context_id: r.try_get("context_id")?,
    })
}

pub async fn get_record(db: &PgPool, id: Uuid) -> Result<Option<FileRecord>> {
    let row = sqlx::query(&format!("SELECT {}{FILE_JOIN} WHERE f.id = $1", file_cols()))
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| record_from(&r)).transpose()?)
}

pub async fn get_records(db: &PgPool, ids: &[Uuid]) -> Result<Vec<FileRecord>> {
    let rows = sqlx::query(&format!("SELECT {}{FILE_JOIN} WHERE f.id = ANY($1)", file_cols()))
        .bind(ids)
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(record_from).collect::<Result<_, _>>()?)
}

/// Whether `viewer` may read the file (shares are checked separately).
pub async fn can_view(state: &AppState, rec: &FileRecord, viewer: Option<&AuthUser>) -> bool {
    if let Some(v) = viewer {
        if v.id == rec.owner_id || v.is_mod() {
            return true;
        }
    }
    if rec.view.deleted_at.is_some() {
        return false;
    }
    if rec.view.purpose == FilePurpose::Avatar {
        return true;
    }
    match rec.view.visibility {
        Visibility::Public => true,
        Visibility::Members if viewer.is_some() => true,
        _ => crate::extensions::file_access(state, rec, viewer).await,
    }
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.trim().replace(['/', '\\', '\0'], "_");
    if name.is_empty() || name == "." || name == ".." {
        return Err(Error::invalid("Bitte gib einen Namen ein."));
    }
    if name.chars().count() > MAX_NAME {
        return Err(Error::invalid("Der Name ist zu lang (max. 200 Zeichen)."));
    }
    Ok(name)
}

// ---------------------------------------------------------------------------
// Quotas
// ---------------------------------------------------------------------------

/// Effective quota in bytes, `None` = unlimited.
pub async fn quota_for(state: &AppState, user_id: Uuid) -> Result<Option<i64>> {
    let own: Option<Option<i64>> = sqlx::query_scalar("SELECT storage_quota_bytes FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?;
    let bytes = match own.flatten() {
        Some(b) => b,
        None => state.settings().default_quota_mb * 1024 * 1024,
    };
    Ok((bytes > 0).then_some(bytes))
}

pub async fn usage(state: &AppState, user_id: Uuid) -> Result<StorageUsage> {
    let r = sqlx::query(
        "SELECT coalesce(sum(size_bytes), 0)::bigint AS used, \
                coalesce(sum(size_bytes) FILTER (WHERE deleted_at IS NOT NULL), 0)::bigint AS trash, \
                count(*) FILTER (WHERE deleted_at IS NULL) AS n \
         FROM files WHERE owner_id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    Ok(StorageUsage {
        used_bytes: r.try_get("used")?,
        quota_bytes: quota_for(state, user_id).await?,
        trash_bytes: r.try_get("trash")?,
        file_count: r.try_get("n")?,
    })
}

// ---------------------------------------------------------------------------
// Ingest
// ---------------------------------------------------------------------------

pub struct UploadMeta {
    pub name: String,
    pub folder_id: Option<Uuid>,
    pub purpose: FilePurpose,
    pub visibility: Visibility,
    pub context_id: Option<Uuid>,
}

fn sniff_mime(name: &str, head: &[u8]) -> String {
    if let Some(kind) = infer::get(head) {
        return kind.mime_type().to_string();
    }
    let guessed = mime_guess::from_path(name).first_or_octet_stream();
    let m = guessed.essence_str().to_string();
    if m == "application/octet-stream" && std::str::from_utf8(head).is_ok() && !head.is_empty() {
        return "text/plain".to_string();
    }
    m
}

/// Checks folder ownership for uploads/moves.
async fn ensure_own_folder(db: &PgPool, user_id: Uuid, folder_id: Option<Uuid>) -> Result<()> {
    if let Some(fid) = folder_id {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM folders WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL)",
        )
        .bind(fid)
        .bind(user_id)
        .fetch_one(db)
        .await?;
        if !ok {
            return Err(Error::NotFound("Ordner"));
        }
    }
    Ok(())
}

/// Validates an upload before streaming starts. May return a visibility
/// the upload must use (e.g. attachments of private chat channels).
pub async fn check_upload(state: &AppState, user: &AuthUser, meta: &UploadMeta) -> Result<Option<Visibility>> {
    state
        .limiter
        .hit(&format!("upload:{}", user.id), 120, StdDuration::from_secs(600))?;
    if meta.purpose == FilePurpose::Drive && !state.settings().modules.files {
        return Err(Error::invalid("Die Dateiablage ist deaktiviert."));
    }
    ensure_own_folder(&state.db, user.id, meta.folder_id).await?;
    crate::extensions::check_upload_context(state, user, meta).await
}

/// Stores a finished temp upload: quota check, blob commit, DB row and
/// thumbnails.
pub async fn ingest(state: &AppState, user: &AuthUser, meta: &UploadMeta, tmp: TempUpload) -> Result<FileView> {
    if let Some(quota) = quota_for(state, user.id).await? {
        let used = usage(state, user.id).await?.used_bytes;
        if used + tmp.size > quota {
            let _ = tokio::fs::remove_file(&tmp.path).await;
            return Err(Error::invalid(format!(
                "Speicherkontingent überschritten ({} von {} belegt).",
                shared::util::format_bytes(used),
                shared::util::format_bytes(quota)
            )));
        }
    }
    let name = validate_name(&meta.name).unwrap_or_else(|_| "Datei".into());
    let mime = sniff_mime(&name, &tmp.head);
    let (key, _new) = state.storage.commit(&tmp).await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO files (id, owner_id, folder_id, name, storage_key, size_bytes, mime_type, sha256, \
                            visibility, purpose, context_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(user.id)
    .bind(meta.folder_id)
    .bind(&name)
    .bind(&key)
    .bind(tmp.size)
    .bind(&mime)
    .bind(&tmp.sha256)
    .bind(meta.visibility.as_str())
    .bind(meta.purpose.as_str())
    .bind(meta.context_id)
    .execute(&state.db)
    .await?;

    if media::is_raster_image(&mime) {
        process_image_file(state, id, &key).await;
    } else if mime.starts_with("video/") || mime.starts_with("audio/") {
        // Video/audio probing can take a while: do it in the background.
        let st = state.clone();
        let key2 = key.clone();
        tokio::spawn(async move { process_av_file(&st, id, &key2).await });
    }
    let view = get_record(&state.db, id)
        .await?
        .map(|r| r.view)
        .ok_or(Error::NotFound("Datei"))?;
    crate::extensions::after_ingest(state, user, meta, &view).await?;
    Ok(view)
}

async fn process_image_file(state: &AppState, id: Uuid, key: &str) {
    let src = state.storage.blob_path(key);
    let thumb = state.storage.thumb_path(key, "thumb");
    let preview = state.storage.thumb_path(key, "preview");
    let result = tokio::task::spawn_blocking(move || media::process_image(&src, &thumb, &preview)).await;
    match result {
        Ok(Ok(info)) => {
            let meta = media::exif_json(&info);
            let _ = sqlx::query(
                "UPDATE files SET width = $2, height = $3, has_thumb = $4, meta = $5 WHERE id = $1",
            )
            .bind(id)
            .bind(info.width as i32)
            .bind(info.height as i32)
            .bind(info.thumbs)
            .bind(meta)
            .execute(&state.db)
            .await;
        }
        Ok(Err(e)) => tracing::info!(error = %e, "image could not be processed"),
        Err(e) => tracing::warn!(error = %e, "image task failed"),
    }
}

async fn process_av_file(state: &AppState, id: Uuid, key: &str) {
    let Some(ffmpeg) = state.ffmpeg.clone() else { return };
    let src = state.storage.blob_path(key);
    let Some(info) = media::probe(&ffmpeg, &src).await else { return };
    let mut has_thumb = false;
    if info.width.is_some() {
        let preview = state.storage.thumb_path(key, "preview");
        if media::video_thumb(&ffmpeg, &src, &preview, info.duration).await {
            let thumb = state.storage.thumb_path(key, "thumb");
            let preview2 = preview.clone();
            has_thumb = tokio::task::spawn_blocking(move || {
                image::open(&preview2)
                    .map(|img| img.thumbnail(media::THUMB_SIZE, media::THUMB_SIZE).save(&thumb).is_ok())
                    .unwrap_or(false)
            })
            .await
            .unwrap_or(false);
        }
    }
    let _ = sqlx::query(
        "UPDATE files SET duration_secs = $2, width = $3, height = $4, has_thumb = $5 WHERE id = $1",
    )
    .bind(id)
    .bind(info.duration)
    .bind(info.width)
    .bind(info.height)
    .bind(has_thumb)
    .execute(&state.db)
    .await;
}

// ---------------------------------------------------------------------------
// Drive listing
// ---------------------------------------------------------------------------

fn folder_from(r: &PgRow) -> Result<FolderView, sqlx::Error> {
    Ok(FolderView {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        parent_id: r.try_get("parent_id")?,
        item_count: r.try_get("item_count")?,
        created_at: r.try_get("created_at")?,
        deleted_at: r.try_get("deleted_at")?,
    })
}

const FOLDER_SELECT: &str = "SELECT d.id, d.name, d.parent_id, d.created_at, d.deleted_at, \
    ((SELECT count(*) FROM folders c WHERE c.parent_id = d.id AND c.deleted_at IS NULL) + \
     (SELECT count(*) FROM files x WHERE x.folder_id = d.id AND x.deleted_at IS NULL)) AS item_count \
    FROM folders d ";

async fn folder_record(db: &PgPool, user_id: Uuid, id: Uuid) -> Result<FolderView> {
    let row = sqlx::query(&format!("{FOLDER_SELECT} WHERE d.id = $1 AND d.owner_id = $2"))
        .bind(id)
        .bind(user_id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Ordner"))?;
    Ok(folder_from(&row)?)
}

async fn breadcrumbs(db: &PgPool, folder_id: Uuid) -> Result<Vec<FolderRef>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "WITH RECURSIVE chain AS ( \
            SELECT id, name, parent_id, 0 AS depth FROM folders WHERE id = $1 \
            UNION ALL \
            SELECT f.id, f.name, f.parent_id, c.depth + 1 FROM folders f JOIN chain c ON f.id = c.parent_id \
            WHERE c.depth < 50 \
         ) SELECT id, name FROM chain ORDER BY depth DESC",
    )
    .bind(folder_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(id, name)| FolderRef { id, name }).collect())
}

fn order_sql(sort: DriveSort, alias: &str) -> String {
    match sort {
        DriveSort::Name => format!("lower({alias}.name)"),
        DriveSort::Newest => format!("{alias}.created_at DESC"),
        DriveSort::Size => format!("{alias}.size_bytes DESC"),
        DriveSort::Type => format!("{alias}.mime_type, lower({alias}.name)"),
    }
}

pub async fn list_drive(
    state: &AppState,
    user: &AuthUser,
    folder_id: Option<Uuid>,
    sort: DriveSort,
    search: Option<String>,
) -> Result<DriveListing> {
    let db = &state.db;
    let folder = match folder_id {
        Some(id) => Some(folder_record(db, user.id, id).await?),
        None => None,
    };
    let crumbs = match folder_id {
        Some(id) => breadcrumbs(db, id).await?,
        None => Vec::new(),
    };
    let search = search.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());

    let (folders, files) = if let Some(q) = &search {
        let folders = sqlx::query(&format!(
            "{FOLDER_SELECT} WHERE d.owner_id = $1 AND d.deleted_at IS NULL AND position($2 in lower(d.name)) > 0 \
             ORDER BY lower(d.name) LIMIT 100"
        ))
        .bind(user.id)
        .bind(q)
        .fetch_all(db)
        .await?;
        let files = sqlx::query(&format!(
            "SELECT {}{FILE_JOIN} WHERE f.owner_id = $1 AND f.deleted_at IS NULL AND f.purpose = 'drive' \
             AND (position($2 in lower(f.name)) > 0 OR f.search @@ plainto_tsquery('simple', $2)) \
             ORDER BY {} LIMIT 300",
            file_cols(),
            order_sql(sort, "f")
        ))
        .bind(user.id)
        .bind(q)
        .fetch_all(db)
        .await?;
        (folders, files)
    } else {
        let folders = sqlx::query(&format!(
            "{FOLDER_SELECT} WHERE d.owner_id = $1 AND d.deleted_at IS NULL \
             AND d.parent_id IS NOT DISTINCT FROM $2 ORDER BY lower(d.name)"
        ))
        .bind(user.id)
        .bind(folder_id)
        .fetch_all(db)
        .await?;
        let files = sqlx::query(&format!(
            "SELECT {}{FILE_JOIN} WHERE f.owner_id = $1 AND f.deleted_at IS NULL AND f.purpose = 'drive' \
             AND f.folder_id IS NOT DISTINCT FROM $2 ORDER BY {}",
            file_cols(),
            order_sql(sort, "f")
        ))
        .bind(user.id)
        .bind(folder_id)
        .fetch_all(db)
        .await?;
        (folders, files)
    };
    Ok(DriveListing {
        folder,
        breadcrumbs: crumbs,
        folders: folders.iter().map(folder_from).collect::<Result<_, _>>()?,
        files: files
            .iter()
            .map(|r| record_from(r).map(|r| r.view))
            .collect::<Result<_, _>>()?,
        usage: usage(state, user.id).await?,
    })
}

/// Trash: deleted top-level items (deleted folders hide their contents).
pub async fn list_trash(state: &AppState, user: &AuthUser) -> Result<DriveListing> {
    let db = &state.db;
    let folders = sqlx::query(&format!(
        "{FOLDER_SELECT} WHERE d.owner_id = $1 AND d.deleted_at IS NOT NULL \
         AND (d.parent_id IS NULL OR NOT EXISTS (SELECT 1 FROM folders p WHERE p.id = d.parent_id AND p.deleted_at IS NOT NULL)) \
         ORDER BY d.deleted_at DESC"
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    let files = sqlx::query(&format!(
        "SELECT {}{FILE_JOIN} WHERE f.owner_id = $1 AND f.deleted_at IS NOT NULL \
         AND (f.folder_id IS NULL OR NOT EXISTS (SELECT 1 FROM folders p WHERE p.id = f.folder_id AND p.deleted_at IS NOT NULL)) \
         ORDER BY f.deleted_at DESC",
        file_cols()
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    Ok(DriveListing {
        folder: None,
        breadcrumbs: Vec::new(),
        folders: folders.iter().map(folder_from).collect::<Result<_, _>>()?,
        files: files
            .iter()
            .map(|r| record_from(r).map(|r| r.view))
            .collect::<Result<_, _>>()?,
        usage: usage(state, user.id).await?,
    })
}

/// All non-deleted folders of the user (for "move to" pickers).
pub async fn folder_tree(db: &PgPool, user: &AuthUser) -> Result<Vec<FolderView>> {
    let rows = sqlx::query(&format!(
        "{FOLDER_SELECT} WHERE d.owner_id = $1 AND d.deleted_at IS NULL ORDER BY lower(d.name)"
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(folder_from).collect::<Result<_, _>>()?)
}

/// Uploads of a given purpose (e.g. chat attachments) for "my uploads".
pub async fn list_by_purpose(db: &PgPool, user: &AuthUser, purpose: FilePurpose, limit: i64) -> Result<Vec<FileView>> {
    let rows = sqlx::query(&format!(
        "SELECT {}{FILE_JOIN} WHERE f.owner_id = $1 AND f.purpose = $2 AND f.deleted_at IS NULL \
         ORDER BY f.created_at DESC LIMIT $3",
        file_cols()
    ))
    .bind(user.id)
    .bind(purpose.as_str())
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(|r| record_from(r).map(|r| r.view)).collect::<Result<_, _>>()?)
}

// ---------------------------------------------------------------------------
// Folder operations
// ---------------------------------------------------------------------------

pub async fn create_folder(db: &PgPool, user: &AuthUser, parent: Option<Uuid>, name: &str) -> Result<FolderView> {
    let name = validate_name(name)?;
    ensure_own_folder(db, user.id, parent).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM folders WHERE owner_id = $1")
        .bind(user.id)
        .fetch_one(db)
        .await?;
    if count >= 5000 {
        return Err(Error::invalid("Zu viele Ordner."));
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO folders (id, owner_id, parent_id, name) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(user.id)
        .bind(parent)
        .bind(&name)
        .execute(db)
        .await?;
    folder_record(db, user.id, id).await
}

pub async fn rename_folder(db: &PgPool, user: &AuthUser, id: Uuid, name: &str) -> Result<()> {
    let name = validate_name(name)?;
    let res = sqlx::query("UPDATE folders SET name = $3, updated_at = now() WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user.id)
        .bind(name)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(Error::NotFound("Ordner"));
    }
    Ok(())
}

pub async fn move_folder(db: &PgPool, user: &AuthUser, id: Uuid, parent: Option<Uuid>) -> Result<()> {
    ensure_own_folder(db, user.id, parent).await?;
    if let Some(p) = parent {
        // Refuse moving a folder into itself or one of its descendants.
        let chain = breadcrumbs(db, p).await?;
        if chain.iter().any(|f| f.id == id) {
            return Err(Error::invalid("Ein Ordner kann nicht in sich selbst verschoben werden."));
        }
    }
    let res = sqlx::query("UPDATE folders SET parent_id = $3, updated_at = now() WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user.id)
        .bind(parent)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(Error::NotFound("Ordner"));
    }
    Ok(())
}

/// Moves a folder and everything below it to the trash.
pub async fn trash_folder(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<()> {
    folder_record(db, user.id, id).await?;
    let mut tx = db.begin().await?;
    sqlx::query(
        "WITH RECURSIVE sub AS (SELECT id FROM folders WHERE id = $1 \
            UNION ALL SELECT f.id FROM folders f JOIN sub s ON f.parent_id = s.id) \
         UPDATE folders SET deleted_at = now() WHERE id IN (SELECT id FROM sub) AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "WITH RECURSIVE sub AS (SELECT id FROM folders WHERE id = $1 \
            UNION ALL SELECT f.id FROM folders f JOIN sub s ON f.parent_id = s.id) \
         UPDATE files SET deleted_at = now() WHERE folder_id IN (SELECT id FROM sub) AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn restore_folder(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<()> {
    folder_record(db, user.id, id).await?;
    let mut tx = db.begin().await?;
    sqlx::query(
        "WITH RECURSIVE sub AS (SELECT id FROM folders WHERE id = $1 \
            UNION ALL SELECT f.id FROM folders f JOIN sub s ON f.parent_id = s.id) \
         UPDATE files SET deleted_at = NULL WHERE folder_id IN (SELECT id FROM sub)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "WITH RECURSIVE sub AS (SELECT id FROM folders WHERE id = $1 \
            UNION ALL SELECT f.id FROM folders f JOIN sub s ON f.parent_id = s.id) \
         UPDATE folders SET deleted_at = NULL WHERE id IN (SELECT id FROM sub)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    // A restored folder whose parent is still in the trash goes to the root.
    sqlx::query(
        "UPDATE folders SET parent_id = NULL WHERE id = $1 AND parent_id IN \
         (SELECT id FROM folders WHERE deleted_at IS NOT NULL)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

async fn own_records(state: &AppState, user: &AuthUser, ids: &[Uuid]) -> Result<Vec<FileRecord>> {
    let recs = get_records(&state.db, ids).await?;
    if recs.len() != ids.len() || recs.iter().any(|r| r.owner_id != user.id && !user.is_mod()) {
        return Err(Error::NotFound("Datei"));
    }
    Ok(recs)
}

pub async fn rename_file(state: &AppState, user: &AuthUser, id: Uuid, name: &str, description: &str) -> Result<()> {
    own_records(state, user, &[id]).await?;
    let name = validate_name(name)?;
    if description.chars().count() > 2000 {
        return Err(Error::invalid("Die Beschreibung ist zu lang."));
    }
    sqlx::query("UPDATE files SET name = $2, description = $3, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(description.trim())
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn move_files(state: &AppState, user: &AuthUser, ids: &[Uuid], folder: Option<Uuid>) -> Result<()> {
    own_records(state, user, ids).await?;
    ensure_own_folder(&state.db, user.id, folder).await?;
    sqlx::query("UPDATE files SET folder_id = $2, updated_at = now() WHERE id = ANY($1)")
        .bind(ids)
        .bind(folder)
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn set_visibility(state: &AppState, user: &AuthUser, ids: &[Uuid], vis: Visibility) -> Result<()> {
    own_records(state, user, ids).await?;
    sqlx::query("UPDATE files SET visibility = $2, updated_at = now() WHERE id = ANY($1)")
        .bind(ids)
        .bind(vis.as_str())
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn trash_files(state: &AppState, user: &AuthUser, ids: &[Uuid]) -> Result<()> {
    let recs = own_records(state, user, ids).await?;
    sqlx::query("UPDATE files SET deleted_at = now() WHERE id = ANY($1) AND deleted_at IS NULL")
        .bind(ids)
        .execute(&state.db)
        .await?;
    for r in recs.iter().filter(|r| r.owner_id != user.id) {
        crate::modlog::record(&state.db, Some(user.id), "delete_content", Some((SubjectType::File, r.view.id)), Some(r.owner_id), &r.view.name).await?;
    }
    Ok(())
}

pub async fn restore_files(state: &AppState, user: &AuthUser, ids: &[Uuid]) -> Result<()> {
    own_records(state, user, ids).await?;
    sqlx::query(
        "UPDATE files SET deleted_at = NULL, \
            folder_id = CASE WHEN folder_id IN (SELECT id FROM folders WHERE deleted_at IS NOT NULL) THEN NULL ELSE folder_id END \
         WHERE id = ANY($1)",
    )
    .bind(ids)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Deletes file rows for good and removes blobs nobody references anymore.
pub(crate) async fn purge_records(state: &AppState, recs: &[FileRecord]) -> Result<()> {
    let ids: Vec<Uuid> = recs.iter().map(|r| r.view.id).collect();
    sqlx::query("DELETE FROM files WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&state.db)
        .await?;
    for r in recs {
        let still_used: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM files WHERE storage_key = $1)")
            .bind(&r.storage_key)
            .fetch_one(&state.db)
            .await?;
        if !still_used {
            state.storage.delete_blob(&r.storage_key).await;
        }
    }
    Ok(())
}

pub async fn purge_files(state: &AppState, user: &AuthUser, ids: &[Uuid]) -> Result<()> {
    let recs = own_records(state, user, ids).await?;
    purge_records(state, &recs).await
}

pub async fn purge_folder(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    folder_record(&state.db, user.id, id).await?;
    let file_ids: Vec<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE sub AS (SELECT id FROM folders WHERE id = $1 \
            UNION ALL SELECT f.id FROM folders f JOIN sub s ON f.parent_id = s.id) \
         SELECT id FROM files WHERE folder_id IN (SELECT id FROM sub)",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let recs = get_records(&state.db, &file_ids).await?;
    purge_records(state, &recs).await?;
    sqlx::query("DELETE FROM folders WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn empty_trash(state: &AppState, user: &AuthUser) -> Result<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM files WHERE owner_id = $1 AND deleted_at IS NOT NULL")
        .bind(user.id)
        .fetch_all(&state.db)
        .await?;
    let recs = get_records(&state.db, &ids).await?;
    purge_records(state, &recs).await?;
    sqlx::query("DELETE FROM folders WHERE owner_id = $1 AND deleted_at IS NOT NULL")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// Background job: purge trash older than [`TRASH_DAYS`].
pub async fn purge_old_trash(state: &AppState) -> Result<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM files WHERE deleted_at < now() - make_interval(days => $1)",
    )
    .bind(TRASH_DAYS as i32)
    .fetch_all(&state.db)
    .await?;
    if !ids.is_empty() {
        let recs = get_records(&state.db, &ids).await?;
        purge_records(state, &recs).await?;
    }
    sqlx::query("DELETE FROM folders WHERE deleted_at < now() - make_interval(days => $1)")
        .bind(TRASH_DAYS as i32)
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn file_info(state: &AppState, viewer: Option<&AuthUser>, id: Uuid) -> Result<FileView> {
    let rec = get_record(&state.db, id).await?.ok_or(Error::NotFound("Datei"))?;
    if !can_view(state, &rec, viewer).await {
        return Err(Error::NotFound("Datei"));
    }
    Ok(rec.view)
}

pub async fn count_download(db: &PgPool, id: Uuid) {
    let _ = sqlx::query("UPDATE files SET download_count = download_count + 1 WHERE id = $1")
        .bind(id)
        .execute(db)
        .await;
}

// ---------------------------------------------------------------------------
// Share links
// ---------------------------------------------------------------------------

fn new_token() -> String {
    let mut bytes = [0u8; 18];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn share_from(r: &PgRow) -> Result<ShareView, sqlx::Error> {
    let file_id: Option<Uuid> = r.try_get("file_id")?;
    let folder_id: Option<Uuid> = r.try_get("folder_id")?;
    let pw: Option<String> = r.try_get("password_hash")?;
    Ok(ShareView {
        id: r.try_get("id")?,
        token: r.try_get("token")?,
        file: file_id
            .map(|id| -> Result<FolderRef, sqlx::Error> {
                Ok(FolderRef {
                    id,
                    name: r.try_get::<Option<String>, _>("file_name")?.unwrap_or_default(),
                })
            })
            .transpose()?,
        folder: folder_id
            .map(|id| -> Result<FolderRef, sqlx::Error> {
                Ok(FolderRef {
                    id,
                    name: r.try_get::<Option<String>, _>("folder_name")?.unwrap_or_default(),
                })
            })
            .transpose()?,
        expires_at: r.try_get("expires_at")?,
        has_password: pw.is_some(),
        max_downloads: r.try_get("max_downloads")?,
        download_count: r.try_get("download_count")?,
        created_at: r.try_get("created_at")?,
    })
}

const SHARE_SELECT: &str = "SELECT s.*, f.name AS file_name, d.name AS folder_name FROM file_shares s \
    LEFT JOIN files f ON f.id = s.file_id LEFT JOIN folders d ON d.id = s.folder_id ";

pub async fn create_share(
    state: &AppState,
    user: &AuthUser,
    file_id: Option<Uuid>,
    folder_id: Option<Uuid>,
    expires_days: Option<i64>,
    password: Option<String>,
    max_downloads: Option<i32>,
) -> Result<ShareView> {
    match (file_id, folder_id) {
        (Some(f), None) => {
            own_records(state, user, &[f]).await?;
        }
        (None, Some(d)) => {
            folder_record(&state.db, user.id, d).await?;
        }
        _ => return Err(Error::invalid("Bitte genau eine Datei oder einen Ordner wählen.")),
    }
    let password_hash = match password.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
        Some(p) => Some(crate::auth::hash_password(&p).await?),
        None => None,
    };
    let expires_at = expires_days.filter(|d| *d > 0).map(|d| Utc::now() + Duration::days(d.min(3650)));
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO file_shares (id, token, file_id, folder_id, created_by, expires_at, password_hash, max_downloads) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(new_token())
    .bind(file_id)
    .bind(folder_id)
    .bind(user.id)
    .bind(expires_at)
    .bind(password_hash)
    .bind(max_downloads.filter(|m| *m > 0))
    .execute(&state.db)
    .await?;
    let row = sqlx::query(&format!("{SHARE_SELECT} WHERE s.id = $1"))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(share_from(&row)?)
}

pub async fn list_shares(db: &PgPool, user: &AuthUser, file_id: Option<Uuid>, folder_id: Option<Uuid>) -> Result<Vec<ShareView>> {
    let rows = sqlx::query(&format!(
        "{SHARE_SELECT} WHERE s.created_by = $1 \
           AND ($2::uuid IS NULL OR s.file_id = $2) AND ($3::uuid IS NULL OR s.folder_id = $3) \
         ORDER BY s.created_at DESC"
    ))
    .bind(user.id)
    .bind(file_id)
    .bind(folder_id)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(share_from).collect::<Result<_, _>>()?)
}

pub async fn revoke_share(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM file_shares WHERE id = $1 AND (created_by = $2 OR $3)")
        .bind(id)
        .bind(user.id)
        .bind(user.is_mod())
        .execute(db)
        .await?;
    Ok(())
}

/// A valid (unexpired, not exhausted) share.
pub struct ActiveShare {
    pub id: Uuid,
    pub token: String,
    pub file_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub owner_id: Uuid,
    pub password_hash: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub async fn active_share(db: &PgPool, token: &str) -> Result<Option<ActiveShare>> {
    let row = sqlx::query(
        "SELECT id, token, file_id, folder_id, created_by, password_hash, expires_at FROM file_shares \
         WHERE token = $1 AND (expires_at IS NULL OR expires_at > now()) \
           AND (max_downloads IS NULL OR download_count < max_downloads)",
    )
    .bind(token)
    .fetch_optional(db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    Ok(Some(ActiveShare {
        id: r.try_get("id")?,
        token: r.try_get("token")?,
        file_id: r.try_get("file_id")?,
        folder_id: r.try_get("folder_id")?,
        owner_id: r.try_get("created_by")?,
        password_hash: r.try_get("password_hash")?,
        expires_at: r.try_get("expires_at")?,
    }))
}

/// Whether the share grants access to the file (directly or via folder).
pub fn share_covers(share: &ActiveShare, rec: &FileRecord) -> bool {
    rec.view.deleted_at.is_none()
        && (share.file_id == Some(rec.view.id)
            || (share.folder_id.is_some() && rec.view.folder_id == share.folder_id))
}

pub async fn count_share_download(db: &PgPool, share_id: Uuid) {
    let _ = sqlx::query("UPDATE file_shares SET download_count = download_count + 1 WHERE id = $1")
        .bind(share_id)
        .execute(db)
        .await;
}

pub async fn share_info(state: &AppState, token: &str, unlocked: bool) -> Result<ShareInfo> {
    let share = active_share(&state.db, token)
        .await?
        .ok_or(Error::NotFound("Freigabe"))?;
    let owner = crate::users::summary(&state.db, share.owner_id)
        .await?
        .ok_or(Error::NotFound("Freigabe"))?;
    let locked = share.password_hash.is_some() && !unlocked;
    let mut info = ShareInfo {
        token: share.token.clone(),
        owner,
        locked,
        file: None,
        folder: None,
        files: Vec::new(),
        expires_at: share.expires_at,
    };
    if locked {
        return Ok(info);
    }
    if let Some(fid) = share.file_id {
        let rec = get_record(&state.db, fid).await?.ok_or(Error::NotFound("Freigabe"))?;
        if rec.view.deleted_at.is_some() {
            return Err(Error::NotFound("Freigabe"));
        }
        info.file = Some(rec.view);
    }
    if let Some(did) = share.folder_id {
        let folder = folder_record(&state.db, share.owner_id, did).await?;
        if folder.deleted_at.is_some() {
            return Err(Error::NotFound("Freigabe"));
        }
        let rows = sqlx::query(&format!(
            "SELECT {}{FILE_JOIN} WHERE f.folder_id = $1 AND f.deleted_at IS NULL ORDER BY lower(f.name)",
            file_cols()
        ))
        .bind(did)
        .fetch_all(&state.db)
        .await?;
        info.files = rows.iter().map(|r| record_from(r).map(|r| r.view)).collect::<Result<_, _>>()?;
        info.folder = Some(folder);
    }
    Ok(info)
}

pub async fn verify_share_password(db: &PgPool, token: &str, password: &str) -> Result<bool> {
    let share = active_share(db, token).await?.ok_or(Error::NotFound("Freigabe"))?;
    match share.password_hash {
        Some(hash) => Ok(crate::auth::verify_password(password, &hash).await),
        None => Ok(true),
    }
}

pub const SESSION_SHARES_KEY: &str = "unlocked_shares";

/// Total storage used (admin statistics).
pub async fn total_usage(db: &PgPool) -> Result<(i64, i64)> {
    Ok(sqlx::query_as("SELECT count(*) FILTER (WHERE deleted_at IS NULL), coalesce(sum(size_bytes), 0)::bigint FROM files")
        .fetch_one(db)
        .await?)
}
