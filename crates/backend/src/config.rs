use std::path::PathBuf;

/// Process configuration from environment variables (`.env` in development).
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// Externally reachable base URL without trailing slash, used for links
    /// in e-mails and feeds (e.g. `https://community.example.org`).
    pub public_url: String,
    /// Root for uploaded files and derived data (thumbnails).
    pub data_dir: PathBuf,
    /// Set the `Secure` flag on cookies (enable behind HTTPS).
    pub cookie_secure: bool,
    /// Trust `X-Forwarded-For` for client IPs (only behind a reverse proxy).
    pub trust_proxy: bool,
    pub smtp: Option<SmtpConfig>,
    pub livekit: Option<LiveKitConfig>,
    /// Optional ffmpeg binary for video thumbnails / durations.
    pub ffmpeg: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
    pub tls: SmtpTls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    /// Implicit TLS (usually port 465).
    Tls,
    /// STARTTLS upgrade (usually port 587).
    StartTls,
    /// Plain text — only for local relays.
    None,
}

#[derive(Debug, Clone)]
pub struct LiveKitConfig {
    /// WebSocket URL handed to browsers, e.g. `wss://livekit.example.org`.
    pub url: String,
    /// HTTP(S) URL for the server API; derived from `url` if not set.
    pub api_url: String,
    pub api_key: String,
    pub api_secret: String,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn env_bool(key: &str, default: bool) -> bool {
    env(key)
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        let database_url = env("DATABASE_URL").expect("DATABASE_URL must be set");
        let site_addr = env("LEPTOS_SITE_ADDR").unwrap_or_else(|| "127.0.0.1:3000".into());
        let public_url = env("PUBLIC_URL")
            .unwrap_or_else(|| format!("http://{site_addr}"))
            .trim_end_matches('/')
            .to_string();
        let data_dir = env("DATA_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("data"));

        let smtp = env("SMTP_HOST").map(|host| {
            let tls = match env("SMTP_TLS").as_deref() {
                Some("tls") => SmtpTls::Tls,
                Some("none") => SmtpTls::None,
                _ => SmtpTls::StartTls,
            };
            let default_port = match tls {
                SmtpTls::Tls => 465,
                SmtpTls::StartTls => 587,
                SmtpTls::None => 25,
            };
            SmtpConfig {
                host,
                port: env("SMTP_PORT").and_then(|p| p.parse().ok()).unwrap_or(default_port),
                username: env("SMTP_USERNAME"),
                password: env("SMTP_PASSWORD"),
                from: env("SMTP_FROM").unwrap_or_else(|| "community@localhost".into()),
                tls,
            }
        });

        let livekit = match (env("LIVEKIT_URL"), env("LIVEKIT_API_KEY"), env("LIVEKIT_API_SECRET")) {
            (Some(url), Some(api_key), Some(api_secret)) => {
                let api_url = env("LIVEKIT_API_URL").unwrap_or_else(|| {
                    url.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1)
                });
                Some(LiveKitConfig {
                    url,
                    api_url: api_url.trim_end_matches('/').to_string(),
                    api_key,
                    api_secret,
                })
            }
            _ => None,
        };

        let ffmpeg = env("FFMPEG_PATH").map(PathBuf::from);

        Config {
            database_url,
            public_url,
            data_dir,
            cookie_secure: env_bool("COOKIE_SECURE", false),
            trust_proxy: env_bool("TRUST_PROXY", false),
            smtp,
            livekit,
            ffmpeg,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.public_url, path)
    }
}
