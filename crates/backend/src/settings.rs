use shared::{AuthUser, SiteSettings};
use sqlx::PgPool;

use crate::error::{Error, Result};
use crate::modlog;
use crate::state::AppState;

pub async fn load(db: &PgPool) -> Result<SiteSettings> {
    let data: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT data FROM site_settings WHERE id")
            .fetch_optional(db)
            .await?;
    Ok(data
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub async fn save(state: &AppState, actor: &AuthUser, mut new: SiteSettings) -> Result<SiteSettings> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    new.site_name = new.site_name.trim().to_string();
    if new.site_name.is_empty() || new.site_name.chars().count() > 80 {
        return Err(Error::invalid("Der Seitenname muss 1–80 Zeichen lang sein."));
    }
    if new.site_description.chars().count() > 300 {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 300 Zeichen)."));
    }
    if new.welcome_md.chars().count() > 20_000 {
        return Err(Error::invalid("Der Willkommenstext ist zu lang."));
    }
    if !(1..=10_240).contains(&new.max_upload_mb) {
        return Err(Error::invalid("Upload-Limit muss zwischen 1 und 10240 MB liegen."));
    }
    if new.default_quota_mb < 0 {
        return Err(Error::invalid("Das Speicherkontingent darf nicht negativ sein."));
    }
    let json = serde_json::to_value(&new).map_err(Error::internal)?;
    sqlx::query("UPDATE site_settings SET data = $1, updated_at = now() WHERE id")
        .bind(json)
        .execute(&state.db)
        .await?;
    state.replace_settings(new.clone());
    modlog::record(&state.db, Some(actor.id), "settings", None, None, "").await?;
    Ok(new)
}

pub async fn render_version(db: &PgPool) -> Result<i32> {
    Ok(sqlx::query_scalar("SELECT render_version FROM site_settings WHERE id")
        .fetch_one(db)
        .await?)
}

pub async fn set_render_version(db: &PgPool, v: i32) -> Result<()> {
    sqlx::query("UPDATE site_settings SET render_version = $1 WHERE id")
        .bind(v)
        .execute(db)
        .await?;
    Ok(())
}
