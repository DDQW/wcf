//! One-off upgrade tasks run at startup.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::Result;
use crate::{forum, markdown, settings};

/// Rebuilds stored HTML when the markdown renderer changed (e.g. after the
/// switch to sanitised output), so old content gets the same treatment.
pub async fn rerender_if_needed(db: &PgPool) -> Result<()> {
    let current = settings::render_version(db).await?;
    if current >= markdown::RENDER_VERSION {
        return Ok(());
    }
    tracing::info!(from = current, to = markdown::RENDER_VERSION, "re-rendering stored markdown");
    let posts = forum::rerender_all(db).await?;
    let bios: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, bio_md FROM users WHERE bio_md <> ''")
        .fetch_all(db)
        .await?;
    for (id, md) in bios {
        let html = markdown::render_mentions(db, &md).await?.html;
        sqlx::query("UPDATE users SET bio_html = $2 WHERE id = $1")
            .bind(id)
            .bind(html)
            .execute(db)
            .await?;
    }
    crate::maintenance_ext::rerender(db).await?;
    settings::set_render_version(db, markdown::RENDER_VERSION).await?;
    tracing::info!(posts, "markdown re-render finished");
    Ok(())
}
