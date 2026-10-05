//! Invite codes (ported from HQTube): when the site is invite-only, new
//! accounts need a valid code. Codes can expire, be used several times and
//! be revoked; claiming happens atomically inside the registration
//! transaction so a failed sign-up never burns a code.

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use rand::Rng;
use shared::admin::{InviteInput, InviteView};
use shared::{AuthUser, NotificationKind, UserSummary};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::users::{opt_user, user_cols, user_from};

/// Members may hold this many usable codes at once.
const MEMBER_ACTIVE_LIMIT: i64 = 5;
/// Unambiguous alphabet (no 0/O, 1/I/L).
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

pub fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    let mut s = String::with_capacity(11);
    for i in 0..10 {
        if i == 5 {
            s.push('-');
        }
        s.push(ALPHABET[rng.gen_range(0..ALPHABET.len())] as char);
    }
    s
}

/// Normalises user input ("abcde fghij", lower case) to the stored form.
pub fn normalize(code: &str) -> String {
    let cleaned: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.len() == 10 {
        format!("{}-{}", &cleaned[..5], &cleaned[5..])
    } else {
        cleaned
    }
}

fn can_invite(state: &AppState, user: &AuthUser) -> bool {
    user.is_mod() || state.settings().members_can_invite
}

pub async fn create(state: &AppState, user: &AuthUser, input: InviteInput) -> Result<Vec<InviteView>> {
    if !can_invite(state, user) {
        return Err(Error::Forbidden);
    }
    state
        .limiter
        .hit(&format!("invite:{}", user.id), 30, StdDuration::from_secs(3600))?;
    let staff = user.is_mod();
    let count = if staff { input.count.clamp(1, 50) } else { 1 };
    let max_uses = if staff { input.max_uses.clamp(1, 1000) } else { 1 };
    let expires_days = match input.expires_days {
        Some(d) => Some(d.clamp(1, 365)),
        None if staff => None,
        None => Some(30),
    };
    if !staff {
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM invite_codes WHERE created_by = $1 AND revoked_at IS NULL \
             AND uses < max_uses AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
        if active >= MEMBER_ACTIVE_LIMIT {
            return Err(Error::invalid(format!(
                "Du hast schon {MEMBER_ACTIVE_LIMIT} offene Einladungen."
            )));
        }
    }
    let note: String = input.note.trim().chars().take(200).collect();
    let expires_at = expires_days.map(|d| Utc::now() + Duration::days(d));
    let mut codes = Vec::new();
    for _ in 0..count {
        // Retry on the (astronomically unlikely) collision.
        for _ in 0..5 {
            let code = generate_code();
            let res = sqlx::query(
                "INSERT INTO invite_codes (code, created_by, expires_at, max_uses, note) VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&code)
            .bind(user.id)
            .bind(expires_at)
            .bind(max_uses)
            .bind(&note)
            .execute(&state.db)
            .await;
            match res {
                Ok(_) => {
                    codes.push(code);
                    break;
                }
                Err(e) => {
                    let e = Error::from(e);
                    if !e.is_unique_violation(None) {
                        return Err(e);
                    }
                }
            }
        }
    }
    list_filtered(&state.db, None, Some(&codes)).await
}

async fn list_filtered(db: &PgPool, creator: Option<Uuid>, codes: Option<&[String]>) -> Result<Vec<InviteView>> {
    let rows = sqlx::query(&format!(
        "SELECT i.code, i.created_at, i.expires_at, i.max_uses, i.uses, i.note, i.revoked_at, {} \
         FROM invite_codes i LEFT JOIN users u ON u.id = i.created_by \
         WHERE ($1::uuid IS NULL OR i.created_by = $1) AND ($2::text[] IS NULL OR i.code = ANY($2)) \
         ORDER BY i.created_at DESC LIMIT 500",
        user_cols("u", "creator")
    ))
    .bind(creator)
    .bind(codes)
    .fetch_all(db)
    .await?;
    let codes: Vec<String> = rows.iter().filter_map(|r| r.try_get("code").ok()).collect();
    let uses = sqlx::query(&format!(
        "SELECT x.code, {} FROM invite_uses x JOIN users u ON u.id = x.user_id WHERE x.code = ANY($1) ORDER BY x.used_at",
        user_cols("u", "u")
    ))
    .bind(&codes)
    .fetch_all(db)
    .await?;
    let mut by_code: HashMap<String, Vec<UserSummary>> = HashMap::new();
    for r in &uses {
        by_code.entry(r.try_get("code")?).or_default().push(user_from(r, "u")?);
    }
    rows.iter()
        .map(|r| {
            let code: String = r.try_get("code")?;
            let revoked: Option<chrono::DateTime<Utc>> = r.try_get("revoked_at")?;
            Ok(InviteView {
                used_by: by_code.remove(&code).unwrap_or_default(),
                code,
                created_by: opt_user(r, "creator")?,
                created_at: r.try_get("created_at")?,
                expires_at: r.try_get("expires_at")?,
                max_uses: r.try_get("max_uses")?,
                uses: r.try_get("uses")?,
                note: r.try_get("note")?,
                revoked: revoked.is_some(),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Error::from)
}

/// Admins/moderators see every code, members their own.
pub async fn list(state: &AppState, user: &AuthUser) -> Result<Vec<InviteView>> {
    if user.is_mod() {
        list_filtered(&state.db, None, None).await
    } else if state.settings().members_can_invite {
        list_filtered(&state.db, Some(user.id), None).await
    } else {
        Err(Error::Forbidden)
    }
}

pub async fn revoke(state: &AppState, user: &AuthUser, code: &str) -> Result<()> {
    let owner: Option<Option<Uuid>> = sqlx::query_scalar("SELECT created_by FROM invite_codes WHERE code = $1")
        .bind(code)
        .fetch_optional(&state.db)
        .await?;
    let owner = owner.ok_or(Error::NotFound("Einladung"))?;
    if owner != Some(user.id) && !user.is_mod() {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE invite_codes SET revoked_at = now() WHERE code = $1 AND revoked_at IS NULL")
        .bind(code)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// Consumes one use of a code inside the registration transaction.
/// Returns the inviter.
pub async fn claim(tx: &mut Transaction<'_, Postgres>, code: &str) -> Result<Option<Uuid>> {
    let code = normalize(code);
    if code.is_empty() {
        return Err(Error::invalid("Für die Registrierung brauchst du einen Einladungscode."));
    }
    let row: Option<(Option<Uuid>,)> = sqlx::query_as(
        "UPDATE invite_codes SET uses = uses + 1 WHERE code = $1 AND revoked_at IS NULL \
         AND uses < max_uses AND (expires_at IS NULL OR expires_at > now()) RETURNING created_by",
    )
    .bind(&code)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((inviter,)) => Ok(inviter),
        None => Err(Error::invalid("Dieser Einladungscode ist ungültig, abgelaufen oder bereits verbraucht.")),
    }
}

pub async fn record_use(tx: &mut Transaction<'_, Postgres>, code: &str, user_id: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO invite_uses (code, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(normalize(code))
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn notify_inviter(state: &AppState, inviter: Uuid, new_user: &AuthUser) {
    notifications::notify(
        state,
        NewNotification {
            user_id: inviter,
            kind: NotificationKind::System,
            actor_id: Some(new_user.id),
            title: format!("{} hat sich mit deiner Einladung registriert", new_user.display_name),
            body: String::new(),
            link: format!("/u/{}", new_user.username),
            subject: None,
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_readable_and_normalised() {
        let c = generate_code();
        assert_eq!(c.len(), 11);
        assert_eq!(&c[5..6], "-");
        assert!(c.chars().filter(|ch| *ch != '-').all(|ch| ALPHABET.contains(&(ch as u8))));
        assert_eq!(normalize("abcde fghjk"), "ABCDE-FGHJK");
        assert_eq!(normalize(" ABCDE-FGHJK "), "ABCDE-FGHJK");
        assert_eq!(normalize(""), "");
    }
}
