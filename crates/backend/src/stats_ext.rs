//! Admin statistics of modules added in later phases.

use shared::AdminStats;
use sqlx::PgPool;

use crate::error::Result;

pub const EXTRA_SQL: &str = "0";

pub async fn fill(db: &PgPool, stats: &mut AdminStats) -> Result<()> {
    let (files, bytes) = crate::files::total_usage(db).await?;
    stats.files = files;
    stats.storage_bytes = bytes;
    stats.chat_messages = crate::chat::total_messages(db).await?;
    stats.blog_posts = crate::blogs::total_posts(db).await?;
    stats.cms_entries = crate::cms::total_entries(db).await?;
    stats.images = crate::gallery::total_images(db).await?;
    Ok(())
}
