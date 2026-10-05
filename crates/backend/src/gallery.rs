//! Gallery: personal and community albums with images stored in the
//! filebase (purpose `gallery`, `context_id` = album).

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use shared::files::{FileView, Visibility};
use shared::gallery::{
    AlbumDetail, AlbumFilter, AlbumInput, AlbumPage, AlbumView, ImageDetail, ImagePage, ImageView,
    ALBUMS_PER_PAGE, IMAGES_PER_PAGE,
};
use shared::{AuthUser, SubjectType};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::files::{self, UploadMeta};
use crate::state::AppState;
use crate::users::{user_cols, user_from};
use crate::{modlog, reactions};

const MAX_TITLE: usize = 120;
const MAX_DESCRIPTION: usize = 5_000;

// ---------------------------------------------------------------------------
// Albums
// ---------------------------------------------------------------------------

fn album_select() -> String {
    format!(
        "SELECT a.id, a.title, a.description, a.visibility, a.is_community, a.created_at, a.updated_at, a.owner_id, \
           (SELECT count(*) FROM gallery_images i WHERE i.album_id = a.id) AS image_count, \
           coalesce((SELECT i.file_id FROM gallery_images i WHERE i.id = a.cover_image_id), \
                    (SELECT i.file_id FROM gallery_images i WHERE i.album_id = a.id \
                     ORDER BY i.position, i.created_at LIMIT 1)) AS cover_file, {} \
         FROM gallery_albums a JOIN users u ON u.id = a.owner_id ",
        user_cols("u", "owner")
    )
}

/// SQL condition: album `a` is visible for the viewer (binds $1 = viewer id
/// or NULL, $2 = logged in, $3 = moderator).
const ALBUM_VISIBLE: &str =
    "(a.visibility = 'public' OR (a.visibility = 'members' AND $2) OR a.owner_id = $1 OR $3)";

fn album_from(r: &PgRow, viewer: Option<&AuthUser>) -> Result<AlbumView, sqlx::Error> {
    let owner_id: Uuid = r.try_get("owner_id")?;
    let visibility = Visibility::parse(&r.try_get::<String, _>("visibility")?);
    let is_community: bool = r.try_get("is_community")?;
    let cover: Option<Uuid> = r.try_get("cover_file")?;
    let can_edit = viewer.is_some_and(|v| v.id == owner_id || v.is_mod());
    Ok(AlbumView {
        id: r.try_get("id")?,
        owner: user_from(r, "owner")?,
        title: r.try_get("title")?,
        description: r.try_get("description")?,
        cover_url: cover.map(|f| format!("/files/{f}/thumb")),
        visibility,
        is_community,
        image_count: r.try_get("image_count")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        can_edit,
        can_upload: can_edit || (is_community && viewer.is_some()),
    })
}

fn album_visible(album: &AlbumView, viewer: Option<&AuthUser>) -> bool {
    match album.visibility {
        Visibility::Public => true,
        Visibility::Members => viewer.is_some(),
        Visibility::Private => viewer.is_some_and(|v| v.id == album.owner.id || v.is_mod()),
    }
}

/// Loads an album the viewer may see.
pub async fn album(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<AlbumView> {
    let row = sqlx::query(&format!("{} WHERE a.id = $1", album_select()))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Album"))?;
    let album = album_from(&row, viewer)?;
    if !album_visible(&album, viewer) {
        return Err(Error::NotFound("Album"));
    }
    Ok(album)
}

pub async fn list_albums(db: &PgPool, viewer: Option<&AuthUser>, filter: AlbumFilter, page: i64) -> Result<AlbumPage> {
    let owner = match filter {
        AlbumFilter::All => None,
        AlbumFilter::User(id) => Some(id),
        AlbumFilter::Mine => Some(viewer.ok_or(Error::Unauthorized)?.id),
    };
    let page = page.max(1);
    let filter_sql = format!("WHERE {ALBUM_VISIBLE} AND ($4::uuid IS NULL OR a.owner_id = $4)");
    let rows = sqlx::query(&format!(
        "{} {filter_sql} ORDER BY a.updated_at DESC LIMIT $5 OFFSET $6",
        album_select()
    ))
    .bind(viewer.map(|v| v.id))
    .bind(viewer.is_some())
    .bind(viewer.is_some_and(|v| v.is_mod()))
    .bind(owner)
    .bind(ALBUMS_PER_PAGE)
    .bind((page - 1) * ALBUMS_PER_PAGE)
    .fetch_all(db)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM gallery_albums a {filter_sql}"))
        .bind(viewer.map(|v| v.id))
        .bind(viewer.is_some())
        .bind(viewer.is_some_and(|v| v.is_mod()))
        .bind(owner)
        .fetch_one(db)
        .await?;
    Ok(AlbumPage {
        items: rows.iter().map(|r| album_from(r, viewer)).collect::<Result<_, _>>()?,
        total,
    })
}

/// Albums the user may add images to (own + community albums).
pub async fn upload_targets(db: &PgPool, user: &AuthUser) -> Result<Vec<AlbumView>> {
    let rows = sqlx::query(&format!(
        "{} WHERE a.owner_id = $1 OR (a.is_community AND a.visibility <> 'private') \
         ORDER BY a.owner_id = $1 DESC, a.updated_at DESC LIMIT 200",
        album_select()
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(|r| album_from(r, Some(user))).collect::<Result<_, _>>()?)
}

fn validate_album(input: &AlbumInput) -> Result<(String, String)> {
    let title = input.title.trim().to_string();
    if title.is_empty() || title.chars().count() > MAX_TITLE {
        return Err(Error::invalid("Der Albumtitel muss 1–120 Zeichen lang sein."));
    }
    let description = input.description.trim().to_string();
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 5.000 Zeichen)."));
    }
    if input.is_community && input.visibility == Visibility::Private {
        return Err(Error::invalid("Community-Alben können nicht privat sein."));
    }
    Ok((title, description))
}

pub async fn create_album(state: &AppState, user: &AuthUser, input: AlbumInput) -> Result<AlbumView> {
    state
        .limiter
        .hit(&format!("album:{}", user.id), 20, std::time::Duration::from_secs(3600))?;
    let (title, description) = validate_album(&input)?;
    // Community albums are a moderator feature unless the owner is a mod.
    let community = input.is_community && user.is_mod();
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO gallery_albums (id, owner_id, title, description, visibility, is_community) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(user.id)
    .bind(&title)
    .bind(&description)
    .bind(input.visibility.as_str())
    .bind(community)
    .execute(&state.db)
    .await?;
    album(&state.db, Some(user), id).await
}

async fn editable_album(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<AlbumView> {
    let a = album(db, Some(user), id).await?;
    if !a.can_edit {
        return Err(Error::Forbidden);
    }
    Ok(a)
}

pub async fn update_album(state: &AppState, user: &AuthUser, id: Uuid, input: AlbumInput) -> Result<AlbumView> {
    let current = editable_album(&state.db, user, id).await?;
    let (title, description) = validate_album(&input)?;
    let community = if user.is_mod() { input.is_community } else { current.is_community };
    if community && input.visibility == Visibility::Private {
        return Err(Error::invalid("Community-Alben können nicht privat sein."));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE gallery_albums SET title = $2, description = $3, visibility = $4, is_community = $5, \
         updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(&title)
    .bind(&description)
    .bind(input.visibility.as_str())
    .bind(community)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE files SET visibility = $2 WHERE id IN (SELECT file_id FROM gallery_images WHERE album_id = $1)",
    )
    .bind(id)
    .bind(input.visibility.as_str())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if current.owner.id != user.id {
        modlog::record(&state.db, Some(user.id), "gallery.album.edit", Some((SubjectType::GalleryAlbum, id)), Some(current.owner.id), &title).await?;
    }
    album(&state.db, Some(user), id).await
}

pub async fn delete_album(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let a = editable_album(&state.db, user, id).await?;
    let image_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM gallery_images WHERE album_id = $1")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    purge_images(state, &image_ids).await?;
    sqlx::query("DELETE FROM gallery_albums WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    reactions::delete_for(&state.db, SubjectType::GalleryAlbum, &[id]).await?;
    if a.owner.id != user.id {
        modlog::record(&state.db, Some(user.id), "gallery.album.delete", None, Some(a.owner.id), &a.title).await?;
    }
    Ok(())
}

pub async fn set_cover(db: &PgPool, user: &AuthUser, album_id: Uuid, image_id: Option<Uuid>) -> Result<()> {
    editable_album(db, user, album_id).await?;
    if let Some(img) = image_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM gallery_images WHERE id = $1 AND album_id = $2)")
            .bind(img)
            .bind(album_id)
            .fetch_one(db)
            .await?;
        if !ok {
            return Err(Error::NotFound("Bild"));
        }
    }
    sqlx::query("UPDATE gallery_albums SET cover_image_id = $2 WHERE id = $1")
        .bind(album_id)
        .bind(image_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Stores a new image order. `ids` must be the album's images.
pub async fn reorder(db: &PgPool, user: &AuthUser, album_id: Uuid, ids: &[Uuid]) -> Result<()> {
    editable_album(db, user, album_id).await?;
    let positions: Vec<i32> = (0..ids.len() as i32).collect();
    sqlx::query(
        "UPDATE gallery_images g SET position = o.pos FROM unnest($2::uuid[], $3::int[]) AS o(id, pos) \
         WHERE g.id = o.id AND g.album_id = $1",
    )
    .bind(album_id)
    .bind(ids)
    .bind(&positions)
    .execute(db)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

fn image_select() -> String {
    format!(
        "SELECT i.id, i.album_id, i.owner_id, i.file_id, i.title, i.description, i.taken_at, i.view_count, \
           i.created_at, f.width, f.height, f.name AS file_name, f.size_bytes, f.meta, \
           (SELECT count(*) FROM reactions r WHERE r.subject_type = 'gallery_image' AND r.subject_id = i.id) AS like_count, \
           (SELECT count(*) FROM comments c WHERE c.subject_type = 'gallery_image' AND c.subject_id = i.id \
              AND c.deleted_at IS NULL) AS comment_count, {} \
         FROM gallery_images i JOIN files f ON f.id = i.file_id JOIN users u ON u.id = i.owner_id ",
        user_cols("u", "owner")
    )
}

const IMAGE_ORDER: &str = "i.position, i.created_at, i.id";

/// Human readable camera details from the stored EXIF subset.
fn exif_labels(meta: &Value) -> Vec<(String, String)> {
    let Some(exif) = meta.get("exif") else { return Vec::new() };
    let get = |k: &str| exif.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let camera = match (get("make"), get("model")) {
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (a, b) => a.or(b),
    };
    if let Some(c) = camera {
        out.push(("Kamera".into(), c));
    }
    for (key, label) in [
        ("lens", "Objektiv"),
        ("focal_length", "Brennweite"),
        ("aperture", "Blende"),
        ("exposure", "Belichtung"),
        ("iso", "ISO"),
    ] {
        if let Some(v) = get(key) {
            out.push((label.into(), v));
        }
    }
    out
}

fn parse_taken_at(meta: &Value) -> Option<DateTime<Utc>> {
    let s = meta.get("exif")?.get("taken_at")?.as_str()?;
    let s = s.trim();
    ["%Y-%m-%d %H:%M:%S", "%Y:%m:%d %H:%M:%S"]
        .iter()
        .find_map(|fmt| NaiveDateTime::parse_from_str(s, fmt).ok())
        .map(|n| n.and_utc())
}

fn image_from(r: &PgRow) -> Result<ImageView, sqlx::Error> {
    let meta: Option<Value> = r.try_get("meta")?;
    Ok(ImageView {
        id: r.try_get("id")?,
        album_id: r.try_get("album_id")?,
        owner: user_from(r, "owner")?,
        file_id: r.try_get("file_id")?,
        title: r.try_get("title")?,
        description: r.try_get("description")?,
        width: r.try_get("width")?,
        height: r.try_get("height")?,
        taken_at: r.try_get("taken_at")?,
        created_at: r.try_get("created_at")?,
        view_count: r.try_get("view_count")?,
        like_count: r.try_get("like_count")?,
        comment_count: r.try_get("comment_count")?,
        exif: meta.as_ref().map(exif_labels).unwrap_or_default(),
        file_name: r.try_get("file_name")?,
        size_bytes: r.try_get("size_bytes")?,
    })
}

pub async fn album_detail(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<AlbumDetail> {
    let album = album(db, viewer, id).await?;
    let rows = sqlx::query(&format!(
        "{} WHERE i.album_id = $1 AND f.deleted_at IS NULL ORDER BY {IMAGE_ORDER} LIMIT 2000",
        image_select()
    ))
    .bind(id)
    .fetch_all(db)
    .await?;
    Ok(AlbumDetail {
        album,
        images: rows.iter().map(image_from).collect::<Result<_, _>>()?,
    })
}

/// Newest images across all visible albums (optionally of one user),
/// keyset-paginated by `created_at|id`.
pub async fn latest_images(
    db: &PgPool,
    viewer: Option<&AuthUser>,
    user: Option<Uuid>,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<ImagePage> {
    let limit = limit.unwrap_or(IMAGES_PER_PAGE).clamp(1, 100);
    let (c_time, c_id) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
        Some((t, id)) => (
            DateTime::parse_from_rfc3339(t).ok().map(|d| d.with_timezone(&Utc)),
            Uuid::parse_str(id).ok(),
        ),
        None => (None, None),
    };
    let rows = sqlx::query(&format!(
        "{} JOIN gallery_albums a ON a.id = i.album_id WHERE {ALBUM_VISIBLE} AND f.deleted_at IS NULL \
         AND ($4::uuid IS NULL OR i.owner_id = $4) \
         AND ($5::timestamptz IS NULL OR (i.created_at, i.id) < ($5, $6)) \
         ORDER BY i.created_at DESC, i.id DESC LIMIT $7",
        image_select()
    ))
    .bind(viewer.map(|v| v.id))
    .bind(viewer.is_some())
    .bind(viewer.is_some_and(|v| v.is_mod()))
    .bind(user)
    .bind(c_time)
    .bind(c_id.unwrap_or_else(Uuid::max))
    .bind(limit + 1)
    .fetch_all(db)
    .await?;
    let mut items: Vec<ImageView> = rows.iter().map(image_from).collect::<Result<_, _>>()?;
    let next_cursor = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items.last().map(|i| format!("{}|{}", i.created_at.to_rfc3339(), i.id))
    } else {
        None
    };
    Ok(ImagePage { items, next_cursor })
}

async fn image_row(db: &PgPool, id: Uuid) -> Result<ImageView> {
    let row = sqlx::query(&format!("{} WHERE i.id = $1 AND f.deleted_at IS NULL", image_select()))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Bild"))?;
    Ok(image_from(&row)?)
}

fn can_edit_image(img: &ImageView, album: &AlbumView, user: Option<&AuthUser>) -> bool {
    user.is_some_and(|u| u.id == img.owner.id || album.can_edit)
}

pub async fn image_detail(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<ImageDetail> {
    let mut image = image_row(db, id).await?;
    let album = album(db, viewer, image.album_id).await?;
    if viewer.is_none_or(|v| v.id != image.owner.id) {
        sqlx::query("UPDATE gallery_images SET view_count = view_count + 1 WHERE id = $1")
            .bind(id)
            .execute(db)
            .await?;
        image.view_count += 1;
    }
    let (prev, next): (Option<Uuid>, Option<Uuid>) = sqlx::query_as(&format!(
        "SELECT prev, next FROM (SELECT i.id, lag(i.id) OVER w AS prev, lead(i.id) OVER w AS next \
           FROM gallery_images i JOIN files f ON f.id = i.file_id \
           WHERE i.album_id = $1 AND f.deleted_at IS NULL WINDOW w AS (ORDER BY {IMAGE_ORDER})) s WHERE s.id = $2"
    ))
    .bind(image.album_id)
    .bind(id)
    .fetch_optional(db)
    .await?
    .unwrap_or((None, None));
    let reactions = reactions::counts_one(db, SubjectType::GalleryImage, id, viewer.map(|v| v.id)).await?;
    let can_edit = can_edit_image(&image, &album, viewer);
    Ok(ImageDetail { image, album, prev, next, reactions, can_edit })
}

pub async fn update_image(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    title: &str,
    description: &str,
    album_id: Option<Uuid>,
) -> Result<()> {
    let image = image_row(&state.db, id).await?;
    let album = album(&state.db, Some(user), image.album_id).await?;
    if !can_edit_image(&image, &album, Some(user)) {
        return Err(Error::Forbidden);
    }
    let title = title.trim();
    let description = description.trim();
    if title.chars().count() > MAX_TITLE {
        return Err(Error::invalid("Der Titel ist zu lang (max. 120 Zeichen)."));
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 5.000 Zeichen)."));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE gallery_images SET title = $2, description = $3 WHERE id = $1")
        .bind(id)
        .bind(title)
        .bind(description)
        .execute(&mut *tx)
        .await?;
    if let Some(target) = album_id.filter(|t| *t != image.album_id) {
        let dest = crate::gallery::album(&state.db, Some(user), target).await?;
        if !dest.can_upload || image.owner.id != user.id && !dest.can_edit {
            return Err(Error::Forbidden);
        }
        sqlx::query(
            "UPDATE gallery_images SET album_id = $2, \
             position = (SELECT coalesce(max(position), -1) + 1 FROM gallery_images WHERE album_id = $2) WHERE id = $1",
        )
        .bind(id)
        .bind(target)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE gallery_albums SET cover_image_id = NULL WHERE id = $1 AND cover_image_id = $2")
            .bind(image.album_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE files SET context_id = $2, visibility = $3 WHERE id = $1")
            .bind(image.file_id)
            .bind(target)
            .bind(dest.visibility.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE gallery_albums SET updated_at = now() WHERE id = $1")
            .bind(target)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn delete_image(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let image = image_row(&state.db, id).await?;
    let album = album(&state.db, Some(user), image.album_id).await?;
    if !can_edit_image(&image, &album, Some(user)) && !user.is_mod() {
        return Err(Error::Forbidden);
    }
    purge_images(state, &[id]).await?;
    if image.owner.id != user.id {
        modlog::record(
            &state.db,
            Some(user.id),
            "gallery.image.delete",
            None,
            Some(image.owner.id),
            &image.display_title(),
        )
        .await?;
    }
    Ok(())
}

/// Removes images together with their files, reactions and comments.
async fn purge_images(state: &AppState, ids: &[Uuid]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let file_ids: Vec<Uuid> = sqlx::query_scalar("SELECT file_id FROM gallery_images WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(&state.db)
        .await?;
    sqlx::query("DELETE FROM gallery_images WHERE id = ANY($1)")
        .bind(ids)
        .execute(&state.db)
        .await?;
    reactions::delete_for(&state.db, SubjectType::GalleryImage, ids).await?;
    sqlx::query("DELETE FROM comments WHERE subject_type = 'gallery_image' AND subject_id = ANY($1)")
        .bind(ids)
        .execute(&state.db)
        .await?;
    let recs = files::get_records(&state.db, &file_ids).await?;
    files::purge_records(state, &recs).await
}

// ---------------------------------------------------------------------------
// Upload integration
// ---------------------------------------------------------------------------

/// Upload check for purpose `gallery`: the album must accept uploads from
/// the user; files take over the album visibility.
pub async fn upload_visibility(db: &PgPool, user: &AuthUser, meta: &UploadMeta) -> Result<Visibility> {
    let album_id = meta.context_id.ok_or_else(|| Error::invalid("Album für das Bild fehlt."))?;
    let a = album(db, Some(user), album_id).await?;
    if !a.can_upload {
        return Err(Error::Forbidden);
    }
    Ok(a.visibility)
}

/// Registers an uploaded file as gallery image. Non-images are removed
/// again.
pub async fn after_ingest(state: &AppState, user: &AuthUser, meta: &UploadMeta, view: &FileView) -> Result<()> {
    let Some(album_id) = meta.context_id else { return Ok(()) };
    let rec = files::get_record(&state.db, view.id).await?.ok_or(Error::NotFound("Datei"))?;
    if !crate::media::is_raster_image(&view.mime_type) || !rec.view.has_thumb {
        files::purge_records(state, &[rec]).await?;
        return Err(Error::invalid("Nur Bilder (JPEG, PNG, GIF, WebP, BMP, TIFF) sind in der Galerie erlaubt."));
    }
    let file_meta: Option<Value> = sqlx::query_scalar("SELECT meta FROM files WHERE id = $1")
        .bind(view.id)
        .fetch_one(&state.db)
        .await?;
    let taken_at = file_meta.as_ref().and_then(parse_taken_at);
    sqlx::query(
        "INSERT INTO gallery_images (id, album_id, owner_id, file_id, taken_at, position) \
         VALUES ($1, $2, $3, $4, $5, (SELECT coalesce(max(position), -1) + 1 FROM gallery_images WHERE album_id = $2))",
    )
    .bind(Uuid::now_v7())
    .bind(album_id)
    .bind(user.id)
    .bind(view.id)
    .bind(taken_at)
    .execute(&state.db)
    .await?;
    sqlx::query("UPDATE gallery_albums SET updated_at = now() WHERE id = $1")
        .bind(album_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// Read access to gallery files follows the album.
pub async fn file_access(db: &PgPool, album_id: Uuid, viewer: Option<&AuthUser>) -> bool {
    album(db, viewer, album_id).await.is_ok()
}

// ---------------------------------------------------------------------------
// Generic subject hooks (reactions, comments, reports)
// ---------------------------------------------------------------------------

pub async fn image_subject_info(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<Option<crate::subjects::SubjectInfo>> {
    let Ok(image) = image_row(db, id).await else { return Ok(None) };
    if album(db, viewer, image.album_id).await.is_err() {
        return Ok(None);
    }
    Ok(Some(crate::subjects::SubjectInfo {
        author_id: Some(image.owner.id),
        title: image.display_title(),
        preview: if image.description.is_empty() { image.display_title() } else { shared::util::truncate_chars(&image.description, 200) },
        link: Some(image.url()),
    }))
}

pub async fn album_subject_info(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<Option<crate::subjects::SubjectInfo>> {
    let Ok(a) = album(db, viewer, id).await else { return Ok(None) };
    Ok(Some(crate::subjects::SubjectInfo {
        author_id: Some(a.owner.id),
        title: a.title.clone(),
        preview: a.title.clone(),
        link: Some(a.url()),
    }))
}

pub async fn image_owner(db: &PgPool, id: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar("SELECT owner_id FROM gallery_images WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?)
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

pub async fn total_images(db: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM gallery_images").fetch_one(db).await?)
}

pub async fn user_image_count(db: &PgPool, user_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM gallery_images WHERE owner_id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await?)
}
