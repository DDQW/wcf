//! Registration, login, sessions, password changes and resets.

use std::time::Duration as StdDuration;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use shared::util::{validate_email, validate_password, validate_username};
use shared::{AuthUser, Role, SubjectType};
use sqlx::{PgPool, Row};
use tower_sessions::Session;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::{modlog, users};

pub const SESSION_USER_KEY: &str = "user_id";

pub async fn hash_password(password: &str) -> Result<String> {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| Error::internal(format!("hash: {e}")))
    })
    .await
    .map_err(Error::internal)?
}

pub async fn verify_password(password: &str, hash: &str) -> bool {
    let password = password.to_string();
    let hash = hash.to_string();
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

pub struct RegisterInput {
    pub username: String,
    pub email: String,
    pub password: String,
    pub display_name: String,
    /// Required when the site is invite-only.
    pub invite_code: String,
}

pub async fn register(state: &AppState, ip: &str, input: RegisterInput) -> Result<AuthUser> {
    state
        .limiter
        .hit(&format!("register:{ip}"), 5, StdDuration::from_secs(3600))?;

    let username = input.username.trim().to_string();
    let email = input.email.trim().to_lowercase();
    validate_username(&username).map_err(Error::invalid)?;
    validate_email(&email).map_err(Error::invalid)?;
    validate_password(&input.password).map_err(Error::invalid)?;
    let display_name = if input.display_name.trim().is_empty() {
        username.clone()
    } else {
        users::validate_display_name(&input.display_name)?
    };

    let user_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&state.db)
        .await?;
    let is_first = user_count == 0;
    if !is_first && !state.settings().registration_open {
        return Err(Error::invalid("Die Registrierung ist derzeit geschlossen."));
    }

    let taken: Option<(bool, bool)> = sqlx::query_as(
        "SELECT coalesce(bool_or(username = $1::citext), false), \
                coalesce(bool_or(email = $2::citext), false) FROM users \
         WHERE username = $1::citext OR email = $2::citext",
    )
    .bind(&username)
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;
    if let Some((name_taken, mail_taken)) = taken {
        if name_taken {
            return Err(Error::invalid("Dieser Benutzername ist bereits vergeben."));
        }
        if mail_taken {
            return Err(Error::invalid("Diese E-Mail-Adresse wird bereits verwendet."));
        }
    }

    let password_hash = hash_password(&input.password).await?;
    let role = if is_first { Role::Admin } else { Role::Member };
    let id = Uuid::now_v7();
    // The code is claimed in the same transaction: a failed sign-up never burns it.
    let needs_invite = !is_first && state.settings().invite_only;
    let mut tx = state.db.begin().await?;
    let inviter = if needs_invite {
        crate::invites::claim(&mut tx, &input.invite_code).await?
    } else {
        None
    };
    let res = sqlx::query(
        "INSERT INTO users (id, username, email, password_hash, display_name, role) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(&username)
    .bind(&email)
    .bind(&password_hash)
    .bind(&display_name)
    .bind(role.as_str())
    .execute(&mut *tx)
    .await;
    if let Err(e) = res {
        let e = Error::from(e);
        if e.is_unique_violation(None) {
            return Err(Error::invalid("Benutzername oder E-Mail-Adresse bereits vergeben."));
        }
        return Err(e);
    }
    if needs_invite {
        crate::invites::record_use(&mut tx, &input.invite_code, id).await?;
    }
    tx.commit().await?;
    let user = users::find_auth_user(&state.db, id)
        .await?
        .ok_or_else(|| Error::internal("user vanished after insert"))?;
    if let Some(inviter) = inviter {
        crate::invites::notify_inviter(state, inviter, &user).await;
    }
    Ok(user)
}

pub async fn login(state: &AppState, ip: &str, username_or_email: &str, password: &str) -> Result<AuthUser> {
    let ident = username_or_email.trim();
    state
        .limiter
        .hit(&format!("login-ip:{ip}"), 20, StdDuration::from_secs(300))?;
    state.limiter.hit(
        &format!("login-user:{}", ident.to_lowercase()),
        8,
        StdDuration::from_secs(300),
    )?;

    let row = sqlx::query(
        "SELECT id, password_hash, is_active, banned_until, ban_reason FROM users \
         WHERE username = $1::citext OR email = $1::citext",
    )
    .bind(ident)
    .fetch_optional(&state.db)
    .await?;
    let invalid = || Error::invalid("Benutzername/E-Mail oder Passwort ist falsch.");
    let Some(row) = row else {
        // Spend comparable time so the response does not reveal existence.
        let _ = verify_password(password, "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$aGFzaA").await;
        return Err(invalid());
    };
    let hash: String = row.try_get("password_hash")?;
    if !verify_password(password, &hash).await {
        return Err(invalid());
    }
    let is_active: bool = row.try_get("is_active")?;
    if !is_active {
        return Err(Error::invalid("Dieses Konto wurde deaktiviert."));
    }
    let banned_until: Option<DateTime<Utc>> = row.try_get("banned_until")?;
    if let Some(until) = banned_until.filter(|t| *t > Utc::now()) {
        let reason: Option<String> = row.try_get("ban_reason")?;
        let when = if until.year_ce().1 > 9000 {
            "dauerhaft".to_string()
        } else {
            format!("bis {}", until.format("%d.%m.%Y %H:%M UTC"))
        };
        return Err(Error::invalid(format!(
            "Dein Konto ist {when} gesperrt{}",
            reason.map(|r| format!(": {r}")).unwrap_or_else(|| ".".into())
        )));
    }
    let id: Uuid = row.try_get("id")?;
    state.limiter.reset(&format!("login-user:{}", ident.to_lowercase()));
    users::find_auth_user(&state.db, id).await?.ok_or_else(invalid)
}

use chrono::Datelike;

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

pub async fn user_from_session(db: &PgPool, session: &Session) -> Result<Option<AuthUser>> {
    let user_id: Option<Uuid> = session
        .get(SESSION_USER_KEY)
        .await
        .map_err(|e| Error::internal(format!("session: {e}")))?;
    match user_id {
        Some(id) => users::find_auth_user(db, id).await,
        None => Ok(None),
    }
}

pub async fn start_session(session: &Session, user_id: Uuid) -> Result<()> {
    // New session id on privilege change prevents session fixation.
    session
        .cycle_id()
        .await
        .map_err(|e| Error::internal(format!("session: {e}")))?;
    session
        .insert(SESSION_USER_KEY, user_id)
        .await
        .map_err(|e| Error::internal(format!("session: {e}")))
}

pub async fn end_session(session: &Session) -> Result<()> {
    session
        .flush()
        .await
        .map_err(|e| Error::internal(format!("session: {e}")))
}

// ---------------------------------------------------------------------------
// Password changes and resets
// ---------------------------------------------------------------------------

pub async fn change_password(db: &PgPool, user_id: Uuid, current: &str, new: &str) -> Result<()> {
    validate_password(new).map_err(Error::invalid)?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await?;
    if !verify_password(current, &hash).await {
        return Err(Error::invalid("Das aktuelle Passwort ist falsch."));
    }
    let new_hash = hash_password(new).await?;
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(new_hash)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn change_email(db: &PgPool, user_id: Uuid, password: &str, email: &str) -> Result<()> {
    let email = email.trim().to_lowercase();
    validate_email(&email).map_err(Error::invalid)?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await?;
    if !verify_password(password, &hash).await {
        return Err(Error::invalid("Das Passwort ist falsch."));
    }
    let res = sqlx::query("UPDATE users SET email = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(&email)
        .execute(db)
        .await;
    match res {
        Ok(_) => Ok(()),
        Err(e) => {
            let e = Error::from(e);
            if e.is_unique_violation(None) {
                Err(Error::invalid("Diese E-Mail-Adresse wird bereits verwendet."))
            } else {
                Err(e)
            }
        }
    }
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Creates a single-use reset token (valid for 2 hours) and returns the
/// raw token; only its hash is stored.
pub async fn create_reset_token(db: &PgPool, user_id: Uuid) -> Result<String> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    sqlx::query(
        "INSERT INTO password_resets (token_hash, user_id, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(token_hash(&token))
    .bind(user_id)
    .bind(Utc::now() + Duration::hours(2))
    .execute(db)
    .await?;
    Ok(token)
}

/// Sends a reset link if the address belongs to an active account. Always
/// succeeds from the caller's perspective so addresses cannot be probed.
pub async fn request_reset(state: &AppState, ip: &str, email: &str) -> Result<()> {
    state
        .limiter
        .hit(&format!("reset:{ip}"), 5, StdDuration::from_secs(3600))?;
    let Some(mailer) = &state.mailer else {
        return Err(Error::invalid(
            "Passwort-Reset per E-Mail ist nicht eingerichtet. Bitte wende dich an einen Admin.",
        ));
    };
    let row: Option<(Uuid, String)> = sqlx::query_as(&format!(
        "SELECT id, display_name FROM users WHERE email = $1::citext AND {}",
        users::ACTIVE_COND
    ))
    .bind(email.trim())
    .fetch_optional(&state.db)
    .await?;
    if let Some((user_id, name)) = row {
        let token = create_reset_token(&state.db, user_id).await?;
        let link = state.config.url(&format!("/reset-password?token={token}"));
        let site = state.settings().site_name;
        let body = format!(
            "Hallo {name},\n\nfür dein Konto bei {site} wurde ein neues Passwort angefordert.\n\
             Über diesen Link kannst du es innerhalb von 2 Stunden festlegen:\n\n{link}\n\n\
             Falls du das nicht warst, kannst du diese E-Mail ignorieren.\n"
        );
        if let Err(e) = mailer.send(email.trim(), &format!("{site}: Passwort zurücksetzen"), body).await {
            tracing::warn!(error = %e, "Reset-Mail konnte nicht gesendet werden");
        }
    }
    Ok(())
}

/// Admin-generated reset link (for sites without SMTP).
pub async fn admin_reset_link(state: &AppState, actor: &AuthUser, user_id: Uuid) -> Result<String> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    let token = create_reset_token(&state.db, user_id).await?;
    modlog::record(
        &state.db,
        Some(actor.id),
        "password_reset",
        Some((SubjectType::User, user_id)),
        Some(user_id),
        "",
    )
    .await?;
    Ok(state.config.url(&format!("/reset-password?token={token}")))
}

pub async fn reset_password(db: &PgPool, token: &str, new_password: &str) -> Result<()> {
    validate_password(new_password).map_err(Error::invalid)?;
    let hash = token_hash(token.trim());
    let row: Option<(Uuid, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT user_id, expires_at, used_at FROM password_resets WHERE token_hash = $1",
    )
    .bind(&hash)
    .fetch_optional(db)
    .await?;
    let invalid = || Error::invalid("Der Link ist ungültig oder abgelaufen.");
    let (user_id, expires_at, used_at) = row.ok_or_else(invalid)?;
    if used_at.is_some() || expires_at < Utc::now() {
        return Err(invalid());
    }
    let new_hash = hash_password(new_password).await?;
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(new_hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE password_resets SET used_at = now() WHERE token_hash = $1")
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM password_resets WHERE user_id = $1 AND token_hash <> $2")
        .bind(user_id)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn purge_expired_resets(db: &PgPool) -> Result<()> {
    sqlx::query("DELETE FROM password_resets WHERE expires_at < now() - interval '1 day'")
        .execute(db)
        .await?;
    Ok(())
}
