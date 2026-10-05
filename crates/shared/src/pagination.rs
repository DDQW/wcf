/// Page size used consistently by DB queries and by callers deciding
/// whether a full page implies more results exist.
pub const DEFAULT_PAGE_SIZE: i64 = 20;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque keyset-pagination cursor: the `(timestamp, id)` of the last row
/// seen by the client. Encoded as base64 so callers never depend on its
/// internal shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub ts: DateTime<Utc>,
    pub id: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error("invalid cursor encoding")]
    Encoding,
}

impl Cursor {
    pub fn new(ts: DateTime<Utc>, id: Uuid) -> Self {
        Self { ts, id }
    }

    pub fn encode(&self) -> String {
        let raw = format!("{}|{}", self.ts.to_rfc3339(), self.id);
        URL_SAFE_NO_PAD.encode(raw)
    }

    pub fn decode(s: &str) -> Result<Self, CursorError> {
        let raw = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| CursorError::Encoding)?;
        let raw = String::from_utf8(raw).map_err(|_| CursorError::Encoding)?;
        let (ts_part, id_part) = raw.split_once('|').ok_or(CursorError::Encoding)?;
        let ts = DateTime::parse_from_rfc3339(ts_part)
            .map_err(|_| CursorError::Encoding)?
            .with_timezone(&Utc);
        let id = Uuid::parse_str(id_part).map_err(|_| CursorError::Encoding)?;
        Ok(Self { ts, id })
    }
}

/// A page of results plus an opaque cursor to fetch the next page, if any.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { items, next_cursor }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrips() {
        let c = Cursor::new(Utc::now(), Uuid::now_v7());
        let encoded = c.encode();
        let decoded = Cursor::decode(&encoded).expect("decode");
        assert_eq!(c.id, decoded.id);
        assert_eq!(c.ts.timestamp_micros(), decoded.ts.timestamp_micros());
    }

    #[test]
    fn cursor_rejects_garbage() {
        assert!(Cursor::decode("not-a-cursor!!").is_err());
    }
}
