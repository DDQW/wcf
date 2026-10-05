use leptos::server_fn::codec::JsonEncoding;
use leptos::server_fn::error::{FromServerFnError, ServerFnErrorErr};
use serde::{Deserialize, Serialize};

/// Error type of every server function. Its `Display` text is shown to the
/// user as-is, so variants carry German messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum AppError {
    #[error("Bitte melde dich an.")]
    Unauthorized,
    #[error("Dafür fehlt dir die Berechtigung.")]
    Forbidden,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("Zu viele Anfragen – bitte warte einen Moment.")]
    RateLimited,
    #[error("{0}")]
    Internal(String),
    #[error("Verbindungsproblem – bitte erneut versuchen.")]
    Transport(String),
}

impl AppError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        AppError::Invalid(msg.into())
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, AppError::NotFound(_))
    }
}

impl FromServerFnError for AppError {
    type Encoder = JsonEncoding;

    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
        AppError::Transport(value.to_string())
    }
}

#[cfg(feature = "ssr")]
impl From<backend::Error> for AppError {
    fn from(e: backend::Error) -> Self {
        use backend::Error as E;
        match e {
            E::Unauthorized => AppError::Unauthorized,
            E::Forbidden => AppError::Forbidden,
            E::NotFound(what) => AppError::NotFound(format!("{what} nicht gefunden.")),
            E::Invalid(msg) => AppError::Invalid(msg),
            E::RateLimited => AppError::RateLimited,
            other => AppError::Internal(other.public_message()),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
