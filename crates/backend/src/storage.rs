//! Blob storage. Files are content-addressed by their SHA-256, so identical
//! uploads share one blob on disk. The local disk is the only backend for
//! now; everything goes through [`Storage`] so an object store can be added
//! later without touching the domain code.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Storage {
    root: PathBuf,
}

/// A fully written temporary upload, ready to be committed.
pub struct TempUpload {
    pub path: PathBuf,
    pub size: i64,
    pub sha256: String,
    /// First bytes of the file, for content sniffing.
    pub head: Vec<u8>,
}

/// Streams chunks into a temp file while hashing and counting.
pub struct UploadWriter {
    file: tokio::fs::File,
    path: PathBuf,
    hasher: Sha256,
    size: i64,
    limit: i64,
    head: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("Die Datei ist zu groß.")]
    TooLarge,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl UploadWriter {
    pub async fn write(&mut self, chunk: &[u8]) -> Result<(), UploadError> {
        self.size += chunk.len() as i64;
        if self.size > self.limit {
            return Err(UploadError::TooLarge);
        }
        if self.head.len() < 8192 {
            let take = (8192 - self.head.len()).min(chunk.len());
            self.head.extend_from_slice(&chunk[..take]);
        }
        self.hasher.update(chunk);
        self.file.write_all(chunk).await?;
        Ok(())
    }

    pub async fn finish(mut self) -> Result<TempUpload, UploadError> {
        self.file.flush().await?;
        self.file.sync_all().await?;
        Ok(TempUpload {
            path: self.path,
            size: self.size,
            sha256: hex::encode(self.hasher.finalize()),
            head: self.head,
        })
    }

    /// Removes the partial file (after an error).
    pub async fn abort(self) {
        drop(self.file);
        let _ = tokio::fs::remove_file(&self.path).await;
    }
}

impl Storage {
    pub fn new(data_dir: &Path) -> std::io::Result<Self> {
        for sub in ["files", "thumbs", "tmp", "media"] {
            std::fs::create_dir_all(data_dir.join(sub))?;
        }
        Ok(Self {
            root: data_dir.to_path_buf(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Storage key (relative path) for content with this hash.
    pub fn key_for(sha256: &str) -> String {
        format!("{}/{}/{}", &sha256[0..2], &sha256[2..4], sha256)
    }

    pub fn blob_path(&self, key: &str) -> PathBuf {
        self.root.join("files").join(key)
    }

    /// Derived renditions: `size` is `thumb` (≤480px) or `preview` (≤1600px).
    pub fn thumb_path(&self, key: &str, size: &str) -> PathBuf {
        self.root
            .join("thumbs")
            .join(format!("{}_{size}.jpg", key.replace('/', "_")))
    }

    pub async fn begin_upload(&self, limit: i64) -> std::io::Result<UploadWriter> {
        let path = self.root.join("tmp").join(Uuid::now_v7().to_string());
        let file = tokio::fs::File::create(&path).await?;
        Ok(UploadWriter {
            file,
            path,
            hasher: Sha256::new(),
            size: 0,
            limit,
            head: Vec::new(),
        })
    }

    /// Moves a temp upload to its content-addressed place. Returns the key
    /// and whether the blob was new.
    pub async fn commit(&self, tmp: &TempUpload) -> std::io::Result<(String, bool)> {
        let key = Self::key_for(&tmp.sha256);
        let dest = self.blob_path(&key);
        if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(&tmp.path).await;
            return Ok((key, false));
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if tokio::fs::rename(&tmp.path, &dest).await.is_err() {
            // Different filesystems: fall back to copy + delete.
            tokio::fs::copy(&tmp.path, &dest).await?;
            let _ = tokio::fs::remove_file(&tmp.path).await;
        }
        Ok((key, true))
    }

    pub async fn delete_blob(&self, key: &str) {
        let _ = tokio::fs::remove_file(self.blob_path(key)).await;
        for size in ["thumb", "preview"] {
            let _ = tokio::fs::remove_file(self.thumb_path(key, size)).await;
        }
    }

    /// Removes temp files older than a day (interrupted uploads).
    pub async fn clean_tmp(&self) {
        let dir = self.root.join("tmp");
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else { return };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                let old = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .is_some_and(|age| age.as_secs() > 86_400);
                if old {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
    }
}
