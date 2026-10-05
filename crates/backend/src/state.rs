use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use shared::{PublicSiteInfo, SiteSettings};
use sqlx::PgPool;

use crate::config::Config;
use crate::error::Result;
use crate::hub::Hub;
use crate::mail::Mailer;
use crate::ratelimit::RateLimiter;
use crate::storage::Storage;
use crate::{markdown, settings};

struct SiteCache {
    settings: SiteSettings,
    welcome_html: String,
}

/// Everything request handlers and server functions need. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub hub: Hub,
    pub limiter: RateLimiter,
    pub mailer: Option<Mailer>,
    pub storage: Storage,
    /// ffmpeg binary, if available (video thumbnails, media library).
    pub ffmpeg: Option<PathBuf>,
    site: Arc<RwLock<SiteCache>>,
}

impl AppState {
    pub async fn new(config: Config, db: PgPool) -> Result<Self> {
        let mailer = match &config.smtp {
            Some(cfg) => match Mailer::new(cfg) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::warn!(error = %e, "SMTP konfiguriert, aber nicht nutzbar");
                    None
                }
            },
            None => None,
        };
        let storage = Storage::new(&config.data_dir)?;
        let ffmpeg = crate::media::detect_ffmpeg(config.ffmpeg.clone()).await;
        match &ffmpeg {
            Some(p) => tracing::info!(path = %p.display(), "ffmpeg gefunden"),
            None => tracing::info!("ffmpeg nicht gefunden – keine Video-Thumbnails"),
        }
        let loaded = settings::load(&db).await?;
        let welcome_html = markdown::render(&loaded.welcome_md);
        Ok(Self {
            db,
            config: Arc::new(config),
            hub: Hub::new(),
            limiter: RateLimiter::new(),
            mailer,
            storage,
            ffmpeg,
            site: Arc::new(RwLock::new(SiteCache {
                settings: loaded,
                welcome_html,
            })),
        })
    }

    pub fn settings(&self) -> SiteSettings {
        self.site.read().expect("settings poisoned").settings.clone()
    }

    pub(crate) fn replace_settings(&self, settings: SiteSettings) {
        let welcome_html = markdown::render(&settings.welcome_md);
        let mut guard = self.site.write().expect("settings poisoned");
        guard.settings = settings;
        guard.welcome_html = welcome_html;
    }

    pub fn site_info(&self) -> PublicSiteInfo {
        let guard = self.site.read().expect("settings poisoned");
        let s = &guard.settings;
        PublicSiteInfo {
            site_name: s.site_name.clone(),
            site_description: s.site_description.clone(),
            registration_open: s.registration_open,
            welcome_html: guard.welcome_html.clone(),
            modules: s.modules.clone(),
            max_upload_mb: s.max_upload_mb,
            calls_configured: self.config.livekit.is_some(),
            password_reset_by_mail: self.mailer.is_some(),
            members_create_channels: s.members_create_channels,
            members_create_watch_rooms: s.members_create_watch_rooms,
            members_create_call_rooms: s.members_create_call_rooms,
        }
    }
}
