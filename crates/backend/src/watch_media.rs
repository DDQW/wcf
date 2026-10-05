//! Media library for watch rooms (port of HQTube's media pipeline):
//! import from the server's `incoming/` folder, ffprobe, direct play for
//! browser-ready files or lossless HLS remux with switchable audio tracks,
//! subtitles → WebVTT, poster frames, a storage cap and a restart-safe job
//! queue. Files are served under `/media/watch/…` to logged-in members.

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

use axum::body::Body;
use axum::extract::{Path as AxPath, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use shared::watch::{
    ImportFile, ImportMode, LibraryOverview, MediaInfo, MediaStatus, MediaTrack, PackageSelection, PlayKind,
    ProbeKind, ProbeTrack, TrackKind,
};
use shared::AuthUser;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use tokio::io::{AsyncBufReadExt, BufReader};
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tower_sessions::Session;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::state::AppState;

const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "webm", "mov", "avi", "ts", "m2ts", "mpg", "mpeg", "wmv", "flv",
];
const DIRECT_EXTENSIONS: &[&str] = &["mp4", "m4v", "webm"];
const DIRECT_VIDEO_CODECS: &[&str] = &["h264", "vp8", "vp9", "av1"];
const DIRECT_AUDIO_CODECS: &[&str] = &["aac", "mp3", "opus", "vorbis", "flac"];
/// Codecs that can be stream-copied into fMP4/CMAF segments and play via MSE.
const FMP4_COPYABLE_VIDEO: &[&str] = &["h264", "av1", "vp9"];
const FMP4_COPYABLE_AUDIO: &[&str] = &["aac", "mp3", "opus", "flac"];
const TEXT_SUB_CODECS: &[&str] = &["subrip", "srt", "ass", "ssa", "webvtt", "mov_text", "text"];
const MAX_IMPORT_DEPTH: usize = 4;
const MAX_IMPORT_FILES: usize = 1000;

// ---------------------------------------------------------------------------
// Paths and limits
// ---------------------------------------------------------------------------

pub fn media_root(state: &AppState) -> PathBuf {
    state.config.data_dir.join("watch").join("media")
}

pub fn import_root(state: &AppState) -> PathBuf {
    let dir = state
        .config
        .watch_import_dir
        .clone()
        .unwrap_or_else(|| state.config.data_dir.join("watch").join("incoming"));
    // Imports store absolute source paths.
    std::path::absolute(&dir).unwrap_or(dir)
}

fn media_dir(state: &AppState, id: Uuid) -> PathBuf {
    media_root(state).join(id.to_string())
}

pub async fn ensure_dirs(state: &AppState) {
    let _ = tokio::fs::create_dir_all(media_root(state)).await;
    let _ = tokio::fs::create_dir_all(import_root(state)).await;
}

async fn dir_size(dir: PathBuf) -> u64 {
    tokio::task::spawn_blocking(move || {
        fn walk(p: &Path) -> u64 {
            let Ok(entries) = std::fs::read_dir(p) else { return 0 };
            entries
                .flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => walk(&e.path()),
                    Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                    _ => 0,
                })
                .sum()
        }
        walk(&dir)
    })
    .await
    .unwrap_or(0)
}

fn fmt_size(bytes: u64) -> String {
    shared::util::format_bytes(bytes as i64)
}

/// Refusal when writing `needed` more bytes would break the storage cap.
async fn quota_exceeded(state: &AppState, needed: u64, exclude: Option<PathBuf>) -> Option<String> {
    let cap = state.config.watch_max_media_bytes?;
    let mut used = dir_size(media_root(state)).await;
    if let Some(dir) = exclude {
        used = used.saturating_sub(dir_size(dir).await);
    }
    if used + needed <= cap {
        return None;
    }
    Some(format!(
        "Speicherlimit der Mediathek erreicht: {} von {} belegt, benötigt werden etwa {} mehr. \
         Lösche Filme oder erhöhe WATCH_MAX_MEDIA_GB.",
        fmt_size(used),
        fmt_size(cap),
        fmt_size(needed)
    ))
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ProbeResult {
    pub duration_ms: Option<i64>,
    pub tracks: Vec<ProbeTrack>,
}

fn ffmpeg(state: &AppState) -> Result<PathBuf> {
    state
        .ffmpeg
        .clone()
        .ok_or_else(|| Error::invalid("ffmpeg ist auf dem Server nicht installiert (FFMPEG_PATH)."))
}

pub fn parse_probe(json: &Value) -> ProbeResult {
    let duration_ms = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite())
        .map(|d| (d * 1000.0).round() as i64);
    let (mut v, mut a, mut s) = (0, 0, 0);
    let mut tracks = Vec::new();
    for st in json.get("streams").and_then(|s| s.as_array()).cloned().unwrap_or_default() {
        let tag = |k: &str| {
            st.get("tags")
                .and_then(|t| t.get(k).or_else(|| t.get(k.to_uppercase())))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        };
        let disposition = |k: &str| st.get("disposition").and_then(|d| d.get(k)).and_then(|x| x.as_i64()) == Some(1);
        let codec = st.get("codec_name").and_then(|c| c.as_str()).unwrap_or("unknown").to_string();
        let int = |k: &str| st.get(k).and_then(|x| x.as_i64()).map(|x| x as i32);
        let kind = match st.get("codec_type").and_then(|c| c.as_str()) {
            Some("video") if !disposition("attached_pic") => ProbeKind::Video,
            Some("audio") => ProbeKind::Audio,
            Some("subtitle") => ProbeKind::Sub,
            _ => continue,
        };
        let type_index = match kind {
            ProbeKind::Video => {
                v += 1;
                v - 1
            }
            ProbeKind::Audio => {
                a += 1;
                a - 1
            }
            ProbeKind::Sub => {
                s += 1;
                s - 1
            }
        };
        tracks.push(ProbeTrack {
            kind,
            type_index,
            text_based: kind == ProbeKind::Sub && TEXT_SUB_CODECS.contains(&codec.as_str()),
            codec,
            lang: tag("language"),
            title: tag("title"),
            is_default: disposition("default"),
            channels: int("channels"),
            width: int("width"),
            height: int("height"),
        });
    }
    ProbeResult { duration_ms, tracks }
}

async fn probe_file(state: &AppState, path: &Path) -> Result<ProbeResult> {
    let bin = crate::media::ffprobe_for(&ffmpeg(state)?);
    let out = tokio::process::Command::new(bin)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| Error::invalid(format!("ffprobe konnte nicht gestartet werden: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Error::invalid(format!("ffprobe fehlgeschlagen: {}", tail(&err, 300))));
    }
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|_| Error::invalid("ffprobe lieferte kein JSON."))?;
    Ok(parse_probe(&json))
}

fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Can this prepared file be handed to browsers byte-for-byte?
pub fn direct_eligibility(path: &Path, probe: &ProbeResult) -> std::result::Result<(), String> {
    let ext = ext_of(path);
    if !DIRECT_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(".{ext} spielt nicht zuverlässig im Browser – nutze HLS-Remux."));
    }
    let video = probe.tracks.iter().find(|t| t.kind == ProbeKind::Video).ok_or("Keine Videospur.")?;
    if !DIRECT_VIDEO_CODECS.contains(&video.codec.as_str()) {
        return Err(format!("Videocodec {} spielt nicht im Browser.", video.codec));
    }
    if let Some(bad) = probe
        .tracks
        .iter()
        .find(|t| t.kind == ProbeKind::Audio && !DIRECT_AUDIO_CODECS.contains(&t.codec.as_str()))
    {
        return Err(format!("Audiocodec {} spielt nicht im Browser.", bad.codec));
    }
    Ok(())
}

fn lang_label(code: Option<&str>) -> Option<String> {
    let code = code?;
    let label = match code.to_lowercase().as_str() {
        "eng" | "en" => "Englisch",
        "ger" | "deu" | "de" => "Deutsch",
        "fre" | "fra" | "fr" => "Französisch",
        "spa" | "es" => "Spanisch",
        "ita" | "it" => "Italienisch",
        "jpn" | "ja" => "Japanisch",
        "kor" | "ko" => "Koreanisch",
        "chi" | "zho" | "zh" => "Chinesisch",
        "rus" | "ru" => "Russisch",
        "por" | "pt" => "Portugiesisch",
        "pol" | "pl" => "Polnisch",
        "nld" | "dut" | "nl" => "Niederländisch",
        "tur" | "tr" => "Türkisch",
        "und" => "Unbekannt",
        other => return Some(other.to_string()),
    };
    Some(label.to_string())
}

// ---------------------------------------------------------------------------
// Import folder
// ---------------------------------------------------------------------------

/// Guesses a display title from a file name:
/// "The.Matrix.1999.1080p.BluRay.x264.mkv" → "The Matrix (1999)".
pub fn title_from_filename(file_name: &str) -> String {
    let stem = match file_name.rfind('.') {
        Some(i) if i > 0 => &file_name[..i],
        _ => file_name,
    };
    let base: String = stem
        .chars()
        .map(|c| if c == '.' || c == '_' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let tidy = |s: &str| -> String {
        s.trim_end_matches([' ', '-', '–', '(', '['])
            .trim_start_matches([' ', '-', '–', ')', ']'])
            .trim()
            .to_string()
    };
    // The last plausible year with a non-empty title before it wins.
    let mut title = base.clone();
    let mut year: Option<String> = None;
    let chars: Vec<char> = base.chars().collect();
    let mut i = 0;
    while i + 4 <= chars.len() {
        let candidate: String = chars[i..i + 4].iter().collect();
        let starts_ok = i == 0 || matches!(chars[i - 1], ' ' | '(' | '[');
        let ends_ok = i + 4 == chars.len() || matches!(chars[i + 4], ' ' | ')' | ']');
        let is_year = candidate.chars().all(|c| c.is_ascii_digit())
            && (candidate.starts_with("19") || candidate.starts_with("20"));
        if starts_ok && ends_ok && is_year {
            let before = tidy(&chars[..i].iter().collect::<String>());
            if !before.is_empty() {
                title = before;
                year = Some(candidate);
            }
        }
        i += 1;
    }
    if year.is_none() {
        const TAGS: &[&str] = &[
            "2160p", "1080p", "720p", "576p", "480p", "4k", "uhd", "bluray", "blu-ray", "bdrip", "brrip", "web-dl",
            "webdl", "web-rip", "webrip", "hdtv", "dvdrip", "dvd", "x264", "x265", "h264", "h265", "h.264", "h.265",
            "hevc", "avc", "av1", "hdr", "hdr10", "dv", "remux", "proper", "repack", "extended", "unrated", "imax",
        ];
        let words: Vec<&str> = base.split(' ').collect();
        if let Some(pos) = words.iter().position(|w| TAGS.contains(&w.to_lowercase().trim_matches(['[', ']', '(', ')']))) {
            if pos > 0 {
                title = words[..pos].join(" ");
            }
        }
    }
    let mut title = tidy(&title);
    if title == title.to_lowercase() {
        title = title
            .split(' ')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    if title.is_empty() {
        return file_name.to_string();
    }
    match year {
        Some(y) => format!("{title} ({y})"),
        None => title,
    }
}

pub fn list_import_files(root: &Path) -> Vec<(PathBuf, String, u64)> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(PathBuf, String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut names: Vec<_> = entries.flatten().collect();
        names.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
        for e in names {
            if out.len() >= MAX_IMPORT_FILES {
                return;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = e.path();
            // Follows symlinks on purpose: linking a library in is the easy setup.
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if meta.is_dir() {
                if depth < MAX_IMPORT_DEPTH {
                    walk(root, &path, depth + 1, out);
                }
            } else if meta.is_file() && VIDEO_EXTENSIONS.contains(&ext_of(&path).as_str()) {
                let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                out.push((path, rel, meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, 1, &mut out);
    out
}

pub async fn import_files(state: &AppState, user: &AuthUser) -> Result<Vec<ImportFile>> {
    require_admin(user)?;
    let root = import_root(state);
    let files = tokio::task::spawn_blocking(move || list_import_files(&root))
        .await
        .unwrap_or_default();
    let known: Vec<(String, String)> =
        sqlx::query_as("SELECT source_path, title FROM watch_media WHERE status <> 'archived'")
            .fetch_all(&state.db)
            .await?;
    Ok(files
        .into_iter()
        .map(|(path, rel, size)| {
            let p = path.to_string_lossy().to_string();
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            ImportFile {
                imported_as: known.iter().find(|(s, _)| *s == p).map(|(_, t)| t.clone()),
                path: p,
                rel_path: rel,
                size: size as i64,
                suggested_title: title_from_filename(&name),
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Library listing
// ---------------------------------------------------------------------------

fn require_admin(user: &AuthUser) -> Result<()> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

const MEDIA_COLS: &str = "id, title, source_type, source_path, size_bytes, duration_ms, status, error, poster_path, probe";

pub async fn tracks_for(db: &PgPool, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<MediaTrack>>> {
    let rows: Vec<(Uuid, String, Option<String>, String, i32, bool)> = sqlx::query_as(
        "SELECT media_id, kind, lang, label, track_index, is_default FROM watch_media_tracks \
         WHERE media_id = ANY($1) ORDER BY media_id, kind, track_index",
    )
    .bind(ids)
    .fetch_all(db)
    .await?;
    let mut map: HashMap<Uuid, Vec<MediaTrack>> = HashMap::new();
    for (id, kind, lang, label, track_index, is_default) in rows {
        map.entry(id).or_default().push(MediaTrack {
            kind: if kind == "audio" { TrackKind::Audio } else { TrackKind::Sub },
            lang,
            label,
            track_index,
            is_default,
        });
    }
    Ok(map)
}

fn info_from(r: &PgRow, tracks: Vec<MediaTrack>, admin: bool) -> Result<MediaInfo, sqlx::Error> {
    let id: Uuid = r.try_get("id")?;
    let status = MediaStatus::parse(&r.try_get::<String, _>("status")?);
    let source_type: String = r.try_get("source_type")?;
    let direct = source_type == "file_local";
    let poster: Option<String> = r.try_get("poster_path")?;
    let source_path: String = r.try_get("source_path")?;
    let probe: Option<Value> = r.try_get("probe")?;
    let show_probe = admin
        && !direct
        && matches!(status, MediaStatus::New | MediaStatus::Error | MediaStatus::Ready);
    Ok(MediaInfo {
        id,
        title: r.try_get("title")?,
        play_kind: (status == MediaStatus::Ready).then_some(if direct { PlayKind::Direct } else { PlayKind::Hls }),
        status,
        error: r.try_get("error")?,
        duration_ms: r.try_get("duration_ms")?,
        progress: (status == MediaStatus::Transcoding).then(|| progress_of(id).unwrap_or(0.0)),
        poster_url: poster.map(|p| format!("/media/watch/{id}/{p}")),
        tracks,
        probe: show_probe
            .then(|| probe.and_then(|p| serde_json::from_value::<ProbeResult>(p).ok()).map(|p| p.tracks))
            .flatten(),
        source_name: admin.then(|| {
            Path::new(&source_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(source_path.clone())
        }),
        size_bytes: r.try_get("size_bytes")?,
    })
}

pub async fn media_info(db: &PgPool, id: Uuid, admin: bool) -> Result<MediaInfo> {
    let row = sqlx::query(&format!("SELECT {MEDIA_COLS} FROM watch_media WHERE id = $1 AND status <> 'archived'"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Film"))?;
    let mut tracks = tracks_for(db, &[id]).await?;
    Ok(info_from(&row, tracks.remove(&id).unwrap_or_default(), admin)?)
}

/// Admins see the whole library; everyone else only ready media.
pub async fn library(state: &AppState, user: &AuthUser) -> Result<LibraryOverview> {
    let admin = user.is_admin();
    let rows = sqlx::query(&format!(
        "SELECT {MEDIA_COLS} FROM watch_media WHERE status <> 'archived' AND ($1 OR status = 'ready') \
         ORDER BY created_at DESC"
    ))
    .bind(admin)
    .fetch_all(&state.db)
    .await?;
    let ids: Vec<Uuid> = rows.iter().filter_map(|r| r.try_get("id").ok()).collect();
    let mut tracks = tracks_for(&state.db, &ids).await?;
    let media = rows
        .iter()
        .map(|r| {
            let id: Uuid = r.try_get("id")?;
            info_from(r, tracks.remove(&id).unwrap_or_default(), admin)
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(LibraryOverview {
        media,
        import_dir: if admin { import_root(state).to_string_lossy().to_string() } else { String::new() },
        used_bytes: if admin { dir_size(media_root(state)).await as i64 } else { 0 },
        cap_bytes: state.config.watch_max_media_bytes.map(|c| c as i64),
        ffmpeg: state.ffmpeg.is_some(),
    })
}

// ---------------------------------------------------------------------------
// Import / package / rename / delete
// ---------------------------------------------------------------------------

pub async fn import(state: &AppState, user: &AuthUser, title: &str, path: &str, mode: ImportMode) -> Result<MediaInfo> {
    require_admin(user)?;
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(Error::invalid("Der Titel muss 1–200 Zeichen lang sein."));
    }
    let path = PathBuf::from(path.trim());
    if !path.is_absolute() || !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(Error::invalid("Datei nicht gefunden (absoluter Pfad auf dem Server nötig)."));
    }
    let probe = probe_file(state, &path).await?;
    let eligibility = direct_eligibility(&path, &probe);
    if mode == ImportMode::Direct {
        if let Err(reason) = &eligibility {
            return Err(Error::invalid(reason.clone()));
        }
    }
    let audio_count = probe.tracks.iter().filter(|t| t.kind == ProbeKind::Audio).count();
    // auto: deliver untouched when possible; multi-audio needs HLS.
    let direct = mode == ImportMode::Direct || (mode == ImportMode::Auto && eligibility.is_ok() && audio_count <= 1);
    let size = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    if !direct {
        if let Some(msg) = quota_exceeded(state, size, None).await {
            return Err(Error::invalid(msg));
        }
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO watch_media (id, title, source_type, source_path, size_bytes, duration_ms, status, probe, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(title)
    .bind(if direct { "file_local" } else { "hls_local" })
    .bind(path.to_string_lossy().to_string())
    .bind(size as i64)
    .bind(probe.duration_ms)
    .bind(if direct { "probing" } else { "new" })
    .bind(serde_json::to_value(&probe).unwrap_or_default())
    .bind(user.id)
    .execute(&state.db)
    .await?;
    if direct {
        enqueue(state, Job::Direct { media_id: id }).await?;
    }
    media_info(&state.db, id, true).await
}

pub async fn package(state: &AppState, user: &AuthUser, id: Uuid, selection: PackageSelection) -> Result<()> {
    require_admin(user)?;
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT source_type, status, source_path FROM watch_media WHERE id = $1 AND status <> 'archived'")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (source_type, status, source_path) = row.ok_or(Error::NotFound("Film"))?;
    if source_type != "hls_local" {
        return Err(Error::invalid("Direct-Play-Filme werden unverändert ausgeliefert."));
    }
    if !matches!(status.as_str(), "new" | "error" | "ready") {
        return Err(Error::invalid("Der Film wird gerade verarbeitet."));
    }
    if selection.audio.is_empty() || selection.audio.len() > 8 || selection.subs.len() > 16 {
        return Err(Error::invalid("Wähle 1–8 Tonspuren und höchstens 16 Untertitel."));
    }
    // Packaging wipes the media dir — never under a live audience.
    if crate::watch_live::is_media_on_stage(state, id) {
        return Err(Error::invalid("Der Film läuft gerade in einem Raum – versuche es danach."));
    }
    let size = tokio::fs::metadata(&source_path).await.map(|m| m.len()).unwrap_or(0);
    if let Some(msg) = quota_exceeded(state, size, Some(media_dir(state, id))).await {
        return Err(Error::invalid(msg));
    }
    sqlx::query("UPDATE watch_media SET status = 'transcoding', error = NULL WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    enqueue(state, Job::Package { media_id: id, selection }).await
}

pub async fn rename(state: &AppState, user: &AuthUser, id: Uuid, title: &str) -> Result<MediaInfo> {
    require_admin(user)?;
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(Error::invalid("Der Titel muss 1–200 Zeichen lang sein."));
    }
    sqlx::query("UPDATE watch_media SET title = $2 WHERE id = $1 AND status <> 'archived'")
        .bind(id)
        .bind(title)
        .execute(&state.db)
        .await?;
    media_info(&state.db, id, true).await
}

/// Deletes a movie and its prepared files. Movies referenced by past shows
/// are archived instead; upcoming shows block deletion.
pub async fn delete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<bool> {
    require_admin(user)?;
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM watch_media WHERE id = $1 AND status <> 'archived'")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    let status = status.ok_or(Error::NotFound("Film"))?;
    if status == "transcoding" || status == "probing" {
        return Err(Error::invalid("Der Film wird noch vorbereitet."));
    }
    if crate::watch_live::is_media_on_stage(state, id) {
        return Err(Error::invalid("Der Film läuft gerade in einem Raum."));
    }
    let refs: Vec<String> = sqlx::query_scalar(
        "SELECT e.status FROM watch_event_items i JOIN watch_events e ON e.id = i.event_id WHERE i.media_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    if refs.iter().any(|s| s == "scheduled" || s == "live") {
        return Err(Error::invalid("Der Film ist in einer geplanten Vorstellung – nimm ihn zuerst aus dem Line-up."));
    }
    let archived = !refs.is_empty();
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM watch_media_tracks WHERE media_id = $1").bind(id).execute(&mut *tx).await?;
    if archived {
        sqlx::query("UPDATE watch_media SET status = 'archived', error = NULL, poster_path = NULL, probe = NULL WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM watch_media WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    let _ = tokio::fs::remove_dir_all(media_dir(state, id)).await;
    Ok(archived)
}

// ---------------------------------------------------------------------------
// Job queue (serial, persisted in watch_jobs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Job {
    Package { media_id: Uuid, selection: PackageSelection },
    Direct { media_id: Uuid },
}

impl Job {
    fn media_id(&self) -> Uuid {
        match self {
            Job::Package { media_id, .. } | Job::Direct { media_id } => *media_id,
        }
    }
}

struct Queue {
    jobs: Mutex<VecDeque<(Uuid, Job)>>,
    notify: tokio::sync::Notify,
    progress: Mutex<HashMap<Uuid, f64>>,
}

fn queue() -> &'static Queue {
    static Q: OnceLock<Queue> = OnceLock::new();
    Q.get_or_init(|| Queue {
        jobs: Mutex::new(VecDeque::new()),
        notify: tokio::sync::Notify::new(),
        progress: Mutex::new(HashMap::new()),
    })
}

fn progress_of(media_id: Uuid) -> Option<f64> {
    queue().progress.lock().ok()?.get(&media_id).copied()
}

async fn enqueue(state: &AppState, job: Job) -> Result<()> {
    let id = Uuid::now_v7();
    let (kind, payload) = match &job {
        Job::Package { selection, .. } => ("package", serde_json::to_value(selection).unwrap_or_default()),
        Job::Direct { .. } => ("direct", Value::Object(Default::default())),
    };
    sqlx::query("INSERT INTO watch_jobs (id, kind, media_id, payload) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(kind)
        .bind(job.media_id())
        .bind(payload)
        .execute(&state.db)
        .await?;
    queue().jobs.lock().expect("queue").push_back((id, job));
    queue().notify.notify_one();
    Ok(())
}

/// Starts the worker and re-queues jobs interrupted by a restart.
pub async fn start(state: AppState) {
    ensure_dirs(&state).await;
    let stale: Vec<(Uuid, String, Uuid, Value)> = sqlx::query_as(
        "SELECT id, kind, media_id, payload FROM watch_jobs WHERE status IN ('queued', 'running') ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for (id, kind, media_id, payload) in stale {
        let job = match kind.as_str() {
            "direct" => Some(Job::Direct { media_id }),
            "package" => serde_json::from_value::<PackageSelection>(payload)
                .ok()
                .map(|selection| Job::Package { media_id, selection }),
            _ => None,
        };
        match job {
            Some(j) => queue().jobs.lock().expect("queue").push_back((id, j)),
            None => {
                let _ = sqlx::query("UPDATE watch_jobs SET status = 'error', error = 'bad payload' WHERE id = $1")
                    .bind(id)
                    .execute(&state.db)
                    .await;
            }
        }
    }
    tokio::spawn(async move {
        loop {
            let next = queue().jobs.lock().expect("queue").pop_front();
            match next {
                Some((job_id, job)) => run_job(&state, job_id, job).await,
                None => queue().notify.notified().await,
            }
        }
    });
}

async fn run_job(state: &AppState, job_id: Uuid, job: Job) {
    let media_id = job.media_id();
    let _ = sqlx::query("UPDATE watch_jobs SET status = 'running', updated_at = now() WHERE id = $1")
        .bind(job_id)
        .execute(&state.db)
        .await;
    let result = match &job {
        Job::Direct { media_id } => prepare_direct(state, *media_id).await,
        Job::Package { media_id, selection } => {
            queue().progress.lock().expect("progress").insert(*media_id, 0.0);
            package_media(state, *media_id, selection, job_id).await
        }
    };
    queue().progress.lock().expect("progress").remove(&media_id);
    match result {
        Ok(()) => {
            let _ = sqlx::query("UPDATE watch_jobs SET status = 'done', progress = 1, updated_at = now() WHERE id = $1")
                .bind(job_id)
                .execute(&state.db)
                .await;
        }
        Err(e) => {
            let message = e.public_message();
            tracing::warn!(media = %media_id, error = %message, "media job failed");
            // Half-written segments are useless and count against the cap.
            let _ = tokio::fs::remove_dir_all(media_dir(state, media_id)).await;
            let _ = sqlx::query("UPDATE watch_jobs SET status = 'error', error = $2, updated_at = now() WHERE id = $1")
                .bind(job_id)
                .bind(&message)
                .execute(&state.db)
                .await;
            let _ = sqlx::query("UPDATE watch_media SET status = 'error', error = $2 WHERE id = $1")
                .bind(media_id)
                .bind(&message)
                .execute(&state.db)
                .await;
        }
    }
}

async fn load_source(state: &AppState, id: Uuid) -> Result<(PathBuf, ProbeResult)> {
    let row: Option<(String, Option<Value>)> = sqlx::query_as("SELECT source_path, probe FROM watch_media WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    let (path, probe) = row.ok_or(Error::NotFound("Film"))?;
    let probe = probe
        .and_then(|p| serde_json::from_value::<ProbeResult>(p).ok())
        .ok_or_else(|| Error::invalid("Der Film wurde nie analysiert."))?;
    Ok((PathBuf::from(path), probe))
}

async fn run_ffmpeg(state: &AppState, args: &[String], cwd: &Path) -> Result<()> {
    let out = tokio::process::Command::new(ffmpeg(state)?)
        .args(["-hide_banner", "-nostdin", "-y"])
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| Error::invalid(format!("ffmpeg konnte nicht gestartet werden: {e}")))?;
    if !out.status.success() {
        return Err(Error::invalid(format!(
            "ffmpeg fehlgeschlagen: {}",
            tail(&String::from_utf8_lossy(&out.stderr), 400)
        )));
    }
    Ok(())
}

/// Extracts text subtitle streams to `subs/s{index}.vtt` in one ffmpeg run.
async fn extract_subtitles(state: &AppState, source: &Path, out_dir: &Path, indexes: &[i32]) -> Result<()> {
    if indexes.is_empty() {
        return Ok(());
    }
    tokio::fs::create_dir_all(out_dir.join("subs")).await.map_err(Error::internal)?;
    let mut args = vec!["-i".to_string(), source.to_string_lossy().to_string()];
    for s in indexes {
        args.extend(["-map".into(), format!("0:s:{s}"), format!("subs/s{s}.vtt")]);
    }
    run_ffmpeg(state, &args, out_dir)
        .await
        .map_err(|e| Error::invalid(format!("Untertitel: {}", e.public_message())))
}

/// Poster frame at ~25 % of the runtime. Best effort.
async fn make_poster(state: &AppState, source: &Path, out_dir: &Path, duration_ms: Option<i64>) -> Option<String> {
    tokio::fs::create_dir_all(out_dir).await.ok()?;
    let at = duration_ms.map(|d| d / 1000 / 4).unwrap_or(10);
    let args: Vec<String> = vec![
        "-ss".into(),
        at.to_string(),
        "-i".into(),
        source.to_string_lossy().to_string(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "scale=640:-2".into(),
        "poster.jpg".into(),
    ];
    run_ffmpeg(state, &args, out_dir).await.ok().map(|_| "poster.jpg".to_string())
}

async fn write_tracks(db: &PgPool, media_id: Uuid, tracks: &[MediaTrack]) -> Result<()> {
    sqlx::query("DELETE FROM watch_media_tracks WHERE media_id = $1").bind(media_id).execute(db).await?;
    for t in tracks {
        sqlx::query(
            "INSERT INTO watch_media_tracks (id, media_id, kind, lang, label, track_index, is_default) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::now_v7())
        .bind(media_id)
        .bind(if t.kind == TrackKind::Audio { "audio" } else { "sub" })
        .bind(&t.lang)
        .bind(&t.label)
        .bind(t.track_index)
        .bind(t.is_default)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Direct play: subtitles + poster next to the untouched source.
async fn prepare_direct(state: &AppState, id: Uuid) -> Result<()> {
    let (source, probe) = load_source(state, id).await?;
    let out_dir = media_dir(state, id);
    tokio::fs::create_dir_all(&out_dir).await.map_err(Error::internal)?;
    let text_subs: Vec<&ProbeTrack> = probe.tracks.iter().filter(|t| t.kind == ProbeKind::Sub && t.text_based).collect();
    let idx: Vec<i32> = text_subs.iter().map(|t| t.type_index).collect();
    extract_subtitles(state, &source, &out_dir, &idx).await?;
    let poster = make_poster(state, &source, &out_dir, probe.duration_ms).await;
    let mut tracks = Vec::new();
    if let Some(a) = probe.tracks.iter().find(|t| t.kind == ProbeKind::Audio) {
        tracks.push(MediaTrack {
            kind: TrackKind::Audio,
            lang: a.lang.clone(),
            label: a.title.clone().or_else(|| lang_label(a.lang.as_deref())).unwrap_or_else(|| "Standardton".into()),
            track_index: 0,
            is_default: true,
        });
    }
    for (i, t) in text_subs.iter().enumerate() {
        tracks.push(MediaTrack {
            kind: TrackKind::Sub,
            lang: t.lang.clone(),
            label: t
                .title
                .clone()
                .or_else(|| lang_label(t.lang.as_deref()))
                .unwrap_or_else(|| format!("Untertitel {}", i + 1)),
            track_index: t.type_index,
            is_default: false,
        });
    }
    write_tracks(&state.db, id, &tracks).await?;
    sqlx::query("UPDATE watch_media SET status = 'ready', error = NULL, poster_path = $2 WHERE id = $1")
        .bind(id)
        .bind(poster)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// HLS VOD with fMP4/CMAF segments: one video variant (copied when
/// browser-playable), each selected audio track as switchable rendition,
/// selected text subtitles as WebVTT.
async fn package_media(state: &AppState, id: Uuid, selection: &PackageSelection, job_id: Uuid) -> Result<()> {
    let (source, probe) = load_source(state, id).await?;
    let audio: Vec<&ProbeTrack> = probe.tracks.iter().filter(|t| t.kind == ProbeKind::Audio).collect();
    let subs: Vec<&ProbeTrack> = probe.tracks.iter().filter(|t| t.kind == ProbeKind::Sub).collect();
    let video = probe
        .tracks
        .iter()
        .find(|t| t.kind == ProbeKind::Video)
        .ok_or_else(|| Error::invalid("Die Quelle hat keine Videospur."))?;
    let audio_sel: Vec<i32> = selection.audio.iter().copied().filter(|i| (*i as usize) < audio.len()).collect();
    let sub_sel: Vec<i32> = selection
        .subs
        .iter()
        .copied()
        .filter(|i| subs.get(*i as usize).is_some_and(|t| t.text_based))
        .collect();
    if audio_sel.is_empty() {
        return Err(Error::invalid("Keine gültige Tonspur gewählt."));
    }
    let out_dir = media_dir(state, id);
    let _ = tokio::fs::remove_dir_all(&out_dir).await;
    tokio::fs::create_dir_all(out_dir.join("v0")).await.map_err(Error::internal)?;
    for n in 0..audio_sel.len() {
        tokio::fs::create_dir_all(out_dir.join(format!("a{n}"))).await.map_err(Error::internal)?;
    }

    extract_subtitles(state, &source, &out_dir, &sub_sel).await?;
    let poster = make_poster(state, &source, &out_dir, probe.duration_ms).await;

    let v_copy = FMP4_COPYABLE_VIDEO.contains(&video.codec.as_str());
    let a_copy = audio_sel
        .iter()
        .all(|a| FMP4_COPYABLE_AUDIO.contains(&audio[*a as usize].codec.as_str()));
    let mut args: Vec<String> = vec![
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        "-i".into(),
        source.to_string_lossy().to_string(),
        "-map".into(),
        "0:v:0".into(),
    ];
    if v_copy {
        args.extend(["-c:v".into(), "copy".into()]);
    } else {
        args.extend(
            ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    for a in &audio_sel {
        args.extend(["-map".into(), format!("0:a:{a}")]);
    }
    if a_copy {
        args.extend(["-c:a".into(), "copy".into()]);
    } else {
        args.extend(["-c:a", "aac", "-b:a", "160k", "-ac", "2"].iter().map(|s| s.to_string()));
    }
    for (n, a) in audio_sel.iter().enumerate() {
        let lang = audio[*a as usize].lang.clone().unwrap_or_else(|| "und".into());
        args.extend([format!("-metadata:s:a:{n}"), format!("language={lang}")]);
    }
    let mut var_map = vec!["v:0,agroup:aud,name:v0".to_string()];
    for (n, a) in audio_sel.iter().enumerate() {
        let mut entry = format!("a:{n},agroup:aud,name:a{n}");
        if n == 0 {
            entry.push_str(",default:yes");
        }
        if let Some(lang) = &audio[*a as usize].lang {
            entry.push_str(&format!(",language:{lang}"));
        }
        var_map.push(entry);
    }
    args.extend(
        [
            "-f", "hls", "-hls_time", "6", "-hls_playlist_type", "vod", "-hls_segment_type", "fmp4", "-hls_flags",
            "independent_segments", "-master_pl_name", "master.m3u8", "-hls_fmp4_init_filename", "init.mp4",
            "-hls_segment_filename", "%v/s%05d.m4s", "-var_stream_map",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.push(var_map.join(" "));
    args.push("%v/index.m3u8".into());

    let mut child = tokio::process::Command::new(ffmpeg(state)?)
        .args(["-hide_banner", "-nostdin", "-y"])
        .args(&args)
        .current_dir(&out_dir)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::invalid(format!("ffmpeg konnte nicht gestartet werden: {e}")))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let err_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut stderr, &mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    });
    let total = probe.duration_ms.unwrap_or(0) as f64;
    let mut lines = BufReader::new(stdout).lines();
    let mut last_db = std::time::Instant::now();
    while let Ok(Some(line)) = lines.next_line().await {
        // out_time_ms is microseconds despite the name.
        if let Some(us) = line.strip_prefix("out_time_ms=").and_then(|v| v.trim().parse::<f64>().ok()) {
            if total > 0.0 {
                let p = (us / 1000.0 / total).clamp(0.0, 0.999);
                queue().progress.lock().expect("progress").insert(id, p);
                if last_db.elapsed().as_secs() >= 2 {
                    last_db = std::time::Instant::now();
                    let _ = sqlx::query("UPDATE watch_jobs SET progress = $2, updated_at = now() WHERE id = $1")
                        .bind(job_id)
                        .bind(p)
                        .execute(&state.db)
                        .await;
                }
            }
        }
    }
    let status = child.wait().await.map_err(Error::internal)?;
    let stderr_text = err_task.await.unwrap_or_default();
    if !status.success() {
        return Err(Error::invalid(format!("ffmpeg fehlgeschlagen: {}", tail(&stderr_text, 600))));
    }

    let mut tracks = Vec::new();
    for (n, a) in audio_sel.iter().enumerate() {
        let t = audio[*a as usize];
        tracks.push(MediaTrack {
            kind: TrackKind::Audio,
            lang: t.lang.clone(),
            label: t
                .title
                .clone()
                .or_else(|| lang_label(t.lang.as_deref()))
                .unwrap_or_else(|| format!("Ton {}", n + 1)),
            // Order in the HLS master = hls.js audioTracks order.
            track_index: n as i32,
            is_default: n == 0,
        });
    }
    for (i, s) in sub_sel.iter().enumerate() {
        let t = subs[*s as usize];
        tracks.push(MediaTrack {
            kind: TrackKind::Sub,
            lang: t.lang.clone(),
            label: t
                .title
                .clone()
                .or_else(|| lang_label(t.lang.as_deref()))
                .unwrap_or_else(|| format!("Untertitel {}", i + 1)),
            // Source subtitle index — matches subs/s{n}.vtt.
            track_index: *s,
            is_default: false,
        });
    }
    write_tracks(&state.db, id, &tracks).await?;
    sqlx::query("UPDATE watch_media SET status = 'ready', error = NULL, poster_path = $2 WHERE id = $1")
        .bind(id)
        .bind(poster)
        .execute(&state.db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------

fn safe_rel(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    if path.is_empty() || path.len() > 200 {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if !s.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')) {
                    return None;
                }
                out.push(s.as_ref());
            }
            _ => return None,
        }
    }
    Some(out)
}

fn content_type(path: &Path) -> &'static str {
    match ext_of(path).as_str() {
        "m3u8" => "application/vnd.apple.mpegurl",
        "m4s" => "video/iso.segment",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "vtt" => "text/vtt; charset=utf-8",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

async fn viewer(state: &AppState, session: &Session) -> Option<AuthUser> {
    crate::auth::user_from_session(&state.db, session).await.ok().flatten()
}

async fn serve_path(path: PathBuf, req: Request, cache: &'static str) -> Response {
    let mime = content_type(&path);
    let mut res = match ServeFile::new(&path).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(never) => match never {},
    };
    if res.status() == StatusCode::NOT_FOUND {
        return (StatusCode::NOT_FOUND, "Nicht gefunden").into_response();
    }
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    res
}

/// `GET /media/watch/{id}/{*path}` — HLS playlists/segments, subtitles, poster.
pub async fn serve_media(
    State(state): State<AppState>,
    session: Session,
    AxPath((id, path)): AxPath<(Uuid, String)>,
    req: Request,
) -> Response {
    if viewer(&state, &session).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Anmeldung erforderlich").into_response();
    }
    let Some(rel) = safe_rel(&path) else {
        return (StatusCode::BAD_REQUEST, "Ungültiger Pfad").into_response();
    };
    // Playlists may be rewritten by a re-package; segments never change.
    let cache = if ext_of(&rel) == "m3u8" { "private, no-cache" } else { "private, max-age=86400" };
    serve_path(media_dir(&state, id).join(rel), req, cache).await
}

/// `GET /media/watch/src/{id}` — direct play of the untouched source.
pub async fn serve_source(
    State(state): State<AppState>,
    session: Session,
    AxPath(id): AxPath<Uuid>,
    req: Request,
) -> Response {
    if viewer(&state, &session).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Anmeldung erforderlich").into_response();
    }
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT source_type, source_path, status FROM watch_media WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    match row {
        Some((t, path, status)) if t == "file_local" && status == "ready" => {
            serve_path(PathBuf::from(path), req, "private, max-age=3600").await
        }
        _ => (StatusCode::NOT_FOUND, "Nicht gefunden").into_response(),
    }
}

/// Stage description of a ready library item.
pub async fn playing_media(db: &PgPool, id: Uuid) -> Result<Option<shared::watch::PlayingMedia>> {
    type Row = (String, String, Option<i64>, String, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT title, source_type, duration_ms, status, poster_path FROM watch_media WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let Some((title, source_type, duration_ms, status, poster)) = row else { return Ok(None) };
    if status != "ready" {
        return Ok(None);
    }
    let direct = source_type == "file_local";
    let tracks = tracks_for(db, &[id]).await?.remove(&id).unwrap_or_default();
    Ok(Some(shared::watch::PlayingMedia {
        key: format!("media:{id}"),
        item_id: None,
        media_id: Some(id),
        title,
        kind: if direct { PlayKind::Direct } else { PlayKind::Hls },
        url: if direct { format!("/media/watch/src/{id}") } else { format!("/media/watch/{id}/master.m3u8") },
        duration_ms,
        tracks,
        poster_url: poster.map(|p| format!("/media/watch/{id}/{p}")),
        subs_base: Some(format!("/media/watch/{id}/subs/")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_filenames() {
        for (input, expected) in [
            ("The.Matrix.1999.1080p.BluRay.x264.mkv", "The Matrix (1999)"),
            ("Alien (1979).mkv", "Alien (1979)"),
            ("Ghost in the Shell (1995) [1080p].mkv", "Ghost in the Shell (1995)"),
            ("Star Wars - Episode IV - A New Hope (1977).mp4", "Star Wars - Episode IV - A New Hope (1977)"),
            ("2001.A.Space.Odyssey.1968.mkv", "2001 A Space Odyssey (1968)"),
            ("Blade.Runner.2049.2017.2160p.mkv", "Blade Runner 2049 (2017)"),
            ("1917.2019.1080p.mkv", "1917 (2019)"),
            ("1917.mkv", "1917"),
            ("Movie.Name.2160p.UHD.mkv", "Movie Name"),
            ("Movie Name - 1080p.mkv", "Movie Name"),
            ("some_home_video.mp4", "Some Home Video"),
            ("the_matrix_1999.mkv", "The Matrix (1999)"),
        ] {
            assert_eq!(title_from_filename(input), expected, "{input}");
        }
    }

    #[test]
    fn import_listing() {
        let root = std::env::temp_dir().join(format!("wcf-import-{}", Uuid::now_v7()));
        std::fs::create_dir_all(root.join("Series").join("S01")).unwrap();
        std::fs::create_dir_all(root.join(".cache")).unwrap();
        std::fs::write(root.join("b.mkv"), "x").unwrap();
        std::fs::write(root.join("a.MP4"), "xy").unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        std::fs::write(root.join("Series").join("S01").join("e01.webm"), "x").unwrap();
        std::fs::write(root.join(".cache").join("hidden.mkv"), "x").unwrap();
        std::fs::write(root.join(".partial.mkv"), "x").unwrap();
        let files = list_import_files(&root);
        let rels: Vec<&str> = files.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(rels, vec!["a.MP4", "b.mkv", "Series/S01/e01.webm"]);
        assert_eq!(files[0].2, 2);
        assert!(list_import_files(&root.join("nope")).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn probe_parsing_and_eligibility() {
        let json = serde_json::json!({
            "format": {"duration": "61.5"},
            "streams": [
                {"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080, "disposition": {"default": 1}},
                {"codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}},
                {"codec_type": "audio", "codec_name": "aac", "channels": 2, "tags": {"language": "ger"}},
                {"codec_type": "audio", "codec_name": "ac3", "channels": 6, "tags": {"language": "eng", "title": "Director"}},
                {"codec_type": "subtitle", "codec_name": "subrip", "tags": {"language": "ger"}},
                {"codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle"}
            ]
        });
        let p = parse_probe(&json);
        assert_eq!(p.duration_ms, Some(61_500));
        assert_eq!(p.tracks.iter().filter(|t| t.kind == ProbeKind::Video).count(), 1);
        let audio: Vec<_> = p.tracks.iter().filter(|t| t.kind == ProbeKind::Audio).collect();
        assert_eq!(audio[1].type_index, 1);
        assert_eq!(audio[1].title.as_deref(), Some("Director"));
        let subs: Vec<_> = p.tracks.iter().filter(|t| t.kind == ProbeKind::Sub).collect();
        assert!(subs[0].text_based && !subs[1].text_based);
        // AC3 audio and the .mkv container both rule out direct play.
        assert!(direct_eligibility(Path::new("/x/film.mkv"), &p).is_err());
        assert!(direct_eligibility(Path::new("/x/film.mp4"), &p).is_err());
        let mut ok = p.clone();
        ok.tracks.retain(|t| t.codec != "ac3");
        assert!(direct_eligibility(Path::new("/x/film.mp4"), &ok).is_ok());
    }

    #[test]
    fn safe_paths() {
        assert!(safe_rel("master.m3u8").is_some());
        assert!(safe_rel("a0/s00001.m4s").is_some());
        assert!(safe_rel("../secret").is_none());
        assert!(safe_rel("/etc/passwd").is_none());
        assert!(safe_rel("subs/s0 .vtt").is_none());
    }
}
