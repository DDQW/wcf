use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Backend error. The `Display` text of the user-facing variants is shown
/// verbatim in the UI, so it is written in German.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Bitte melde dich an.")]
    Unauthorized,
    #[error("Dafür fehlt dir die Berechtigung.")]
    Forbidden,
    #[error("{0} nicht gefunden.")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("Zu viele Anfragen – bitte warte einen Moment.")]
    RateLimited,
    #[error("Datenbankfehler: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Dateisystemfehler: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Internal(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }

    pub fn internal(msg: impl ToString) -> Self {
        Error::Internal(msg.to_string())
    }

    /// Postgres unique-constraint violation, optionally for a specific constraint.
    pub fn is_unique_violation(&self, constraint: Option<&str>) -> bool {
        match self {
            Error::Db(sqlx::Error::Database(e)) => {
                e.code().as_deref() == Some("23505")
                    && constraint.is_none_or(|c| e.constraint() == Some(c))
            }
            _ => false,
        }
    }

    /// Whether the message is safe and meaningful to show to end users.
    /// Infrastructure errors are logged and replaced by a generic text.
    pub fn is_user_facing(&self) -> bool {
        !matches!(self, Error::Db(_) | Error::Io(_) | Error::Internal(_))
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden => StatusCode::FORBIDDEN,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Error::Db(_) | Error::Io(_) | Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Text for the client: the real message for user-facing errors, a
    /// generic one (after logging) for everything else.
    pub fn public_message(&self) -> String {
        if self.is_user_facing() {
            self.to_string()
        } else {
            tracing::error!(error = %self, "internal error");
            "Interner Serverfehler – bitte versuche es später erneut.".to_string()
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = serde_json::json!({ "error": self.public_message() });
        (status, axum::Json(body)).into_response()
    }
}
