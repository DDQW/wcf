//! HTTP endpoints of the filebase: multipart upload and file delivery with
//! range support (video streaming).

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Multipart, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use shared::files::{FileKind, FilePurpose, UploadResult, Visibility};
use shared::AuthUser;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tower_sessions::Session;
use uuid::Uuid;

use crate::error::Error;
use crate::files::{self, FileRecord, UploadMeta};
use crate::state::AppState;
use crate::storage::UploadError;

pub async fn upload(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<UploadResult>, Error> {
    if !crate::http::same_origin(&headers) {
        return Err(Error::Forbidden);
    }
    let user = crate::auth::user_from_session(&state.db, &session)
        .await?
        .ok_or(Error::Unauthorized)?;
    let limit = state.settings().max_upload_mb * 1024 * 1024;

    let mut folder_id = None;
    let mut purpose = FilePurpose::Drive;
    let mut visibility = None;
    let mut context_id = None;
    let mut checked = false;
    let mut result = UploadResult::default();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| Error::invalid(format!("Upload fehlerhaft: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if field.file_name().is_none() {
            let value = field.text().await.unwrap_or_default();
            match name.as_str() {
                "folder_id" => folder_id = Uuid::parse_str(value.trim()).ok(),
                "purpose" => purpose = FilePurpose::parse(value.trim()),
                "visibility" => visibility = Some(Visibility::parse(value.trim())),
                "context_id" => context_id = Uuid::parse_str(value.trim()).ok(),
                _ => {}
            }
            continue;
        }
        let file_name = field.file_name().unwrap_or("Datei").to_string();
        let meta = UploadMeta {
            name: file_name.clone(),
            folder_id,
            purpose,
            visibility: visibility.unwrap_or(match purpose {
                FilePurpose::Avatar | FilePurpose::Embed | FilePurpose::Blog => Visibility::Public,
                _ => Visibility::Private,
            }),
            context_id,
        };
        if !checked {
            files::check_upload(&state, &user, &meta).await?;
            checked = true;
        }
        let mut writer = state.storage.begin_upload(limit).await?;
        let mut failed = None;
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(e) = writer.write(&chunk).await {
                        failed = Some(match e {
                            UploadError::TooLarge => format!(
                                "{file_name}: zu groß (max. {} MB).",
                                state.settings().max_upload_mb
                            ),
                            UploadError::Io(e) => format!("{file_name}: Schreibfehler ({e})."),
                        });
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    failed = Some(format!("{file_name}: Übertragung abgebrochen ({e})."));
                    break;
                }
            }
        }
        if let Some(msg) = failed {
            writer.abort().await;
            result.errors.push(msg);
            // The rest of an oversized part cannot be skipped cleanly.
            break;
        }
        let tmp = match writer.finish().await {
            Ok(t) => t,
            Err(e) => {
                result.errors.push(format!("{file_name}: {e}"));
                continue;
            }
        };
        match files::ingest(&state, &user, &meta, tmp).await {
            Ok(view) => result.files.push(view),
            Err(e) => result.errors.push(format!("{file_name}: {}", e.public_message())),
        }
    }
    Ok(Json(result))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Inline,
    Download,
    Thumb,
    Preview,
}

async fn unlocked_shares(session: &Session) -> Vec<String> {
    session
        .get::<Vec<String>>(files::SESSION_SHARES_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Resolves access: returns the file plus the share used (if any).
async fn authorize(
    state: &AppState,
    session: &Session,
    id: Uuid,
    share_token: Option<&String>,
) -> Result<(FileRecord, Option<files::ActiveShare>, Option<AuthUser>), Error> {
    let rec = files::get_record(&state.db, id).await?.ok_or(Error::NotFound("Datei"))?;
    let viewer = crate::auth::user_from_session(&state.db, session).await.ok().flatten();
    if files::can_view(state, &rec, viewer.as_ref()).await {
        return Ok((rec, None, viewer));
    }
    if let Some(token) = share_token {
        if let Some(share) = files::active_share(&state.db, token).await? {
            let unlocked = share.password_hash.is_none() || unlocked_shares(session).await.contains(token);
            if unlocked && files::share_covers(&share, &rec) {
                return Ok((rec, Some(share), viewer));
            }
        }
    }
    Err(Error::NotFound("Datei"))
}

fn content_disposition(kind: &str, name: &str) -> HeaderValue {
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => encoded.push(b as char),
            _ => encoded.push_str(&format!("%{b:02X}")),
        }
    }
    HeaderValue::from_str(&format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

/// Inline display is only allowed for types that cannot run script.
fn inline_safe(mime: &str) -> bool {
    let kind = FileKind::of(mime);
    match kind {
        FileKind::Image => mime != "image/svg+xml",
        FileKind::Video | FileKind::Audio | FileKind::Pdf => true,
        FileKind::Text => true,
        _ => false,
    }
}

async fn serve(state: AppState, session: Session, id: Uuid, params: HashMap<String, String>, req: Request, mode: Mode) -> Response {
    let (rec, share, viewer) = match authorize(&state, &session, id, params.get("s")).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let (path, mime) = match mode {
        Mode::Thumb | Mode::Preview => {
            let size = if mode == Mode::Thumb { "thumb" } else { "preview" };
            let p = state.storage.thumb_path(&rec.storage_key, size);
            if !rec.view.has_thumb || !tokio::fs::try_exists(&p).await.unwrap_or(false) {
                return (StatusCode::NOT_FOUND, "Keine Vorschau").into_response();
            }
            (p, "image/jpeg".to_string())
        }
        _ => (state.storage.blob_path(&rec.storage_key), rec.view.mime_type.clone()),
    };
    let download = mode == Mode::Download || (mode == Mode::Inline && !inline_safe(&mime));
    if mode == Mode::Download || (download && req.headers().get(header::RANGE).is_none()) {
        files::count_download(&state.db, rec.view.id).await;
        if let Some(s) = &share {
            files::count_share_download(&state.db, s.id).await;
        }
    }
    let mut res = match ServeFile::new(&path).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(never) => match never {},
    };
    if res.status() == StatusCode::NOT_FOUND {
        return (StatusCode::NOT_FOUND, "Datei fehlt im Speicher").into_response();
    }
    let h = res.headers_mut();
    let served_mime = if FileKind::of(&mime) == FileKind::Text && !download {
        "text/plain; charset=utf-8".to_string()
    } else {
        mime
    };
    if let Ok(v) = HeaderValue::from_str(&served_mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(
        header::CONTENT_DISPOSITION,
        content_disposition(if download { "attachment" } else { "inline" }, &rec.view.name),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'unsafe-inline'"),
    );
    let public = rec.view.visibility == Visibility::Public || rec.view.purpose == FilePurpose::Avatar;
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if public && viewer.is_none() && share.is_none() {
            "public, max-age=86400"
        } else {
            "private, max-age=3600"
        }),
    );
    res
}

pub async fn serve_inline(State(state): State<AppState>, session: Session, Path(id): Path<Uuid>, Query(q): Query<HashMap<String, String>>, req: Request) -> Response {
    serve(state, session, id, q, req, Mode::Inline).await
}

pub async fn serve_download(State(state): State<AppState>, session: Session, Path(id): Path<Uuid>, Query(q): Query<HashMap<String, String>>, req: Request) -> Response {
    serve(state, session, id, q, req, Mode::Download).await
}

pub async fn serve_thumb(State(state): State<AppState>, session: Session, Path(id): Path<Uuid>, Query(q): Query<HashMap<String, String>>, req: Request) -> Response {
    serve(state, session, id, q, req, Mode::Thumb).await
}

pub async fn serve_preview(State(state): State<AppState>, session: Session, Path(id): Path<Uuid>, Query(q): Query<HashMap<String, String>>, req: Request) -> Response {
    serve(state, session, id, q, req, Mode::Preview).await
}
