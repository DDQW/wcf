//! Media helpers: image thumbnails + EXIF, and ffmpeg/ffprobe for videos.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageDecoder, ImageReader};
use serde_json::{json, Map, Value};

pub const THUMB_SIZE: u32 = 480;
pub const PREVIEW_SIZE: u32 = 1600;

pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub exif: Value,
    pub thumbs: bool,
}

/// Image formats we decode for thumbnails.
pub fn is_raster_image(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/bmp" | "image/tiff"
    )
}

fn save_jpeg(img: &DynamicImage, max: u32, dest: &Path) -> image::ImageResult<()> {
    let resized = if img.width() > max || img.height() > max {
        img.thumbnail(max, max)
    } else {
        img.clone()
    };
    // Flatten transparency onto white so JPEG does not turn it black.
    let rgba = resized.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as f32 / 255.0;
        let blend = |c: u8| (c as f32 * a + 255.0 * (1.0 - a)).round() as u8;
        rgb.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    let file = std::fs::File::create(dest)?;
    let mut writer = std::io::BufWriter::new(file);
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 82);
    rgb.write_with_encoder(encoder)
}

/// Decodes an image, honours EXIF orientation and writes the thumb and
/// preview renditions. Blocking — call from `spawn_blocking`.
pub fn process_image(src: &Path, thumb: &Path, preview: &Path) -> Result<ImageInfo, String> {
    let mut reader = ImageReader::open(src)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(768 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let (width, height) = (img.width(), img.height());
    let mut thumbs = save_jpeg(&img, THUMB_SIZE, thumb).is_ok();
    thumbs &= save_jpeg(&img, PREVIEW_SIZE, preview).is_ok();
    Ok(ImageInfo {
        width,
        height,
        exif: read_exif(src),
        thumbs,
    })
}

/// Camera metadata worth showing. GPS data is deliberately ignored.
pub fn read_exif(src: &Path) -> Value {
    let Ok(file) = std::fs::File::open(src) else { return Value::Null };
    let mut reader = BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return Value::Null;
    };
    let mut out = Map::new();
    let fields = [
        (exif::Tag::Make, "make"),
        (exif::Tag::Model, "model"),
        (exif::Tag::LensModel, "lens"),
        (exif::Tag::DateTimeOriginal, "taken_at"),
        (exif::Tag::ExposureTime, "exposure"),
        (exif::Tag::FNumber, "aperture"),
        (exif::Tag::PhotographicSensitivity, "iso"),
        (exif::Tag::FocalLength, "focal_length"),
    ];
    for (tag, key) in fields {
        if let Some(f) = exif.get_field(tag, exif::In::PRIMARY) {
            let value = f.display_value().with_unit(&exif).to_string();
            let value = value.trim_matches('"').trim().to_string();
            if !value.is_empty() {
                out.insert(key.to_string(), Value::String(value));
            }
        }
    }
    if out.is_empty() {
        Value::Null
    } else {
        Value::Object(out)
    }
}

/// `ffprobe` next to the configured `ffmpeg` binary.
pub fn ffprobe_for(ffmpeg: &Path) -> PathBuf {
    let name = ffmpeg
        .file_name()
        .map(|n| n.to_string_lossy().replace("ffmpeg", "ffprobe"))
        .unwrap_or_else(|| "ffprobe".into());
    ffmpeg.with_file_name(name)
}

async fn runs(candidate: &Path) -> bool {
    tokio::process::Command::new(candidate)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// winget installs ffmpeg under `%LOCALAPPDATA%/Microsoft/WinGet/Packages/*/*/bin`,
/// often without updating the PATH of already running shells.
fn winget_candidates() -> Vec<PathBuf> {
    let Some(local) = std::env::var_os("LOCALAPPDATA") else { return Vec::new() };
    let root = PathBuf::from(local).join("Microsoft").join("WinGet");
    let mut out = vec![root.join("Links").join("ffmpeg.exe")];
    if let Ok(packages) = std::fs::read_dir(root.join("Packages")) {
        let mut found: Vec<PathBuf> = packages
            .flatten()
            .filter(|p| p.file_name().to_string_lossy().to_lowercase().contains("ffmpeg"))
            .filter_map(|p| std::fs::read_dir(p.path()).ok())
            .flat_map(|dirs| dirs.flatten().map(|d| d.path().join("bin").join("ffmpeg.exe")))
            .filter(|p| p.is_file())
            .collect();
        // Prefer the newest build.
        found.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        found.reverse();
        out.extend(found);
    }
    out
}

/// Finds a usable ffmpeg (configured path, PATH, or a winget install).
pub async fn detect_ffmpeg(configured: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = configured {
        return runs(&path).await.then_some(path);
    }
    let default = PathBuf::from("ffmpeg");
    if runs(&default).await {
        return Some(default);
    }
    if cfg!(windows) {
        for candidate in winget_candidates() {
            if runs(&candidate).await {
                return Some(candidate);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Default)]
pub struct ProbeInfo {
    pub duration: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub raw: Value,
}

/// Runs `ffprobe -show_format -show_streams` and extracts the basics.
pub async fn probe(ffmpeg: &Path, src: &Path) -> Option<ProbeInfo> {
    let output = tokio::process::Command::new(ffprobe_for(ffmpeg))
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(src)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw: Value = serde_json::from_slice(&output.stdout).ok()?;
    let duration = raw
        .pointer("/format/duration")
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok());
    let video = raw
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|streams| streams.iter().find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("video")));
    let width = video.and_then(|v| v.get("width")).and_then(|w| w.as_i64()).map(|w| w as i32);
    let height = video.and_then(|v| v.get("height")).and_then(|h| h.as_i64()).map(|h| h as i32);
    Some(ProbeInfo {
        duration,
        width,
        height,
        raw,
    })
}

/// Grabs a frame (at 10% of the duration, max 30 s) as JPEG thumbnail.
pub async fn video_thumb(ffmpeg: &Path, src: &Path, dest: &Path, duration: Option<f64>) -> bool {
    let at = duration.map(|d| (d * 0.1).min(30.0)).unwrap_or(1.0);
    tokio::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-ss", &format!("{at:.2}"), "-i"])
        .arg(src)
        .args(["-frames:v", "1", "-vf", &format!("scale='min({PREVIEW_SIZE},iw)':-2"), "-q:v", "3"])
        .arg(dest)
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn exif_json(info: &ImageInfo) -> Value {
    if info.exif.is_null() {
        json!({})
    } else {
        json!({ "exif": info.exif })
    }
}
