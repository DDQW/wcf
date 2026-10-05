use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use shared::user::{
    MemberItem, MemberSort, NotifyPrefs, ProfileStats, ProfileUpdate, ProfileView, SettingsView,
};
use shared::{AdminUserRow, AuthUser, Page, Role, SubjectType, Theme, UserSummary};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::{markdown, modlog};

// ---------------------------------------------------------------------------
// Row helpers shared by every module that embeds users in its views.
// ---------------------------------------------------------------------------

/// URL of a user's avatar thumbnail (served by the file routes).
pub fn avatar_url(file_id: Option<Uuid>) -> Option<String> {
    file_id.map(|id| format!("/files/{id}/thumb"))
}

/// SELECT list for a joined user table alias, with prefixed column names
/// that [`user_from`] / [`opt_user`] read back.
pub fn user_cols(alias: &str, prefix: &str) -> String {
    format!(
        "{alias}.id AS {prefix}_id, {alias}.username::text AS {prefix}_username, \
         {alias}.display_name AS {prefix}_display_name, {alias}.avatar_file_id AS {prefix}_avatar, \
         {alias}.role AS {prefix}_role"
    )
}

pub fn user_from(r: &PgRow, prefix: &str) -> Result<UserSummary, sqlx::Error> {
    let role: String = r.try_get(format!("{prefix}_role").as_str())?;
    Ok(UserSummary {
        id: r.try_get(format!("{prefix}_id").as_str())?,
        username: r.try_get(format!("{prefix}_username").as_str())?,
        display_name: r.try_get(format!("{prefix}_display_name").as_str())?,
        avatar_url: avatar_url(r.try_get(format!("{prefix}_avatar").as_str())?),
        role: Role::parse(&role).unwrap_or_default(),
    })
}

/// Like [`user_from`] for LEFT JOINs: `None` when the user id is NULL.
pub fn opt_user(r: &PgRow, prefix: &str) -> Result<Option<UserSummary>, sqlx::Error> {
    let id: Option<Uuid> = r.try_get(format!("{prefix}_id").as_str())?;
    if id.is_none() {
        return Ok(None);
    }
    user_from(r, prefix).map(Some)
}

fn auth_user_from(r: &PgRow) -> Result<AuthUser, sqlx::Error> {
    let role: String = r.try_get("role")?;
    let theme: String = r.try_get("theme")?;
    Ok(AuthUser {
        id: r.try_get("id")?,
        username: r.try_get("username")?,
        display_name: r.try_get("display_name")?,
        role: Role::parse(&role).unwrap_or_default(),
        avatar_url: avatar_url(r.try_get("avatar_file_id")?),
        theme: Theme::parse(&theme),
    })
}

const AUTH_COLS: &str =
    "id, username::text AS username, display_name, role, avatar_file_id, theme";

/// SQL condition for "may log in right now".
pub const ACTIVE_COND: &str = "is_active AND (banned_until IS NULL OR banned_until <= now())";

pub async fn find_auth_user(db: &PgPool, id: Uuid) -> Result<Option<AuthUser>> {
    let row = sqlx::query(&format!(
        "SELECT {AUTH_COLS} FROM users WHERE id = $1 AND {ACTIVE_COND}"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| auth_user_from(&r)).transpose()?)
}

pub async fn summary(db: &PgPool, id: Uuid) -> Result<Option<UserSummary>> {
    let row = sqlx::query(&format!("SELECT {} FROM users u WHERE u.id = $1", user_cols("u", "u")))
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| user_from(&r, "u")).transpose()?)
}

pub async fn summaries(db: &PgPool, ids: &[Uuid]) -> Result<HashMap<Uuid, UserSummary>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(&format!(
        "SELECT {} FROM users u WHERE u.id = ANY($1)",
        user_cols("u", "u")
    ))
    .bind(ids)
    .fetch_all(db)
    .await?;
    let mut map = HashMap::new();
    for r in rows {
        let u = user_from(&r, "u")?;
        map.insert(u.id, u);
    }
    Ok(map)
}

pub async fn find_by_username(db: &PgPool, username: &str) -> Result<Option<UserSummary>> {
    let row = sqlx::query(&format!(
        "SELECT {} FROM users u WHERE u.username = $1::citext",
        user_cols("u", "u")
    ))
    .bind(username)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| user_from(&r, "u")).transpose()?)
}

pub async fn notify_prefs(db: &PgPool, user_id: Uuid) -> Result<NotifyPrefs> {
    let v: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT notify_prefs FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
}

pub async fn touch_last_seen(db: &PgPool, user_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE users SET last_seen_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Prefix search for @mention autocompletion.
pub async fn search_prefix(db: &PgPool, prefix: &str, limit: i64) -> Result<Vec<UserSummary>> {
    let prefix = prefix.trim().trim_start_matches('@').to_lowercase();
    if prefix.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(&format!(
        "SELECT {} FROM users u WHERE u.is_active AND \
         (starts_with(lower(u.username::text), $1) OR starts_with(lower(u.display_name), $1)) \
         ORDER BY u.last_seen_at DESC NULLS LAST, u.username LIMIT $2",
        user_cols("u", "u")
    ))
    .bind(&prefix)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(|r| user_from(r, "u")).collect::<Result<_, _>>()?)
}

// ---------------------------------------------------------------------------
// Profiles and member list
// ---------------------------------------------------------------------------

pub async fn profile_stats(db: &PgPool, user_id: Uuid) -> Result<ProfileStats> {
    let r = sqlx::query(
        "SELECT \
           (SELECT count(*) FROM forum_topics WHERE author_id = $1 AND NOT is_deleted) AS topics, \
           (SELECT count(*) FROM forum_posts WHERE author_id = $1 AND NOT is_deleted) AS posts, \
           (SELECT count(*) FROM reactions r JOIN forum_posts p \
              ON r.subject_type = 'forum_post' AND r.subject_id = p.id \
              WHERE p.author_id = $1) + \
           (SELECT count(*) FROM reactions r JOIN blog_posts b \
              ON r.subject_type = 'blog_post' AND r.subject_id = b.id \
              WHERE b.author_id = $1) + \
           (SELECT count(*) FROM reactions r JOIN gallery_images g \
              ON r.subject_type = 'gallery_image' AND r.subject_id = g.id \
              WHERE g.owner_id = $1) AS reactions_received",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(ProfileStats {
        topics: r.try_get("topics")?,
        posts: r.try_get("posts")?,
        blog_posts: crate::blogs::user_post_count(db, user_id).await?,
        images: crate::gallery::user_image_count(db, user_id).await?,
        reactions_received: r.try_get("reactions_received")?,
    })
}

pub async fn profile(db: &PgPool, username: &str, viewer: Option<&AuthUser>) -> Result<ProfileView> {
    let r = sqlx::query(&format!(
        "SELECT {}, u.bio_html, u.location, u.website, u.created_at, u.last_seen_at, \
                u.banned_until, u.ban_reason, u.is_active \
         FROM users u WHERE u.username = $1::citext",
        user_cols("u", "u")
    ))
    .bind(username)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Benutzer"))?;
    let user = user_from(&r, "u")?;
    let is_active: bool = r.try_get("is_active")?;
    let staff = viewer.is_some_and(|v| v.is_mod());
    if !is_active && !staff {
        return Err(Error::NotFound("Benutzer"));
    }
    let stats = profile_stats(db, user.id).await?;
    let banned_until: Option<DateTime<Utc>> = r.try_get("banned_until")?;
    let banned_until = banned_until.filter(|t| *t > Utc::now());
    Ok(ProfileView {
        user,
        bio_html: r.try_get("bio_html")?,
        location: r.try_get("location")?,
        website: r.try_get("website")?,
        created_at: r.try_get("created_at")?,
        last_seen_at: r.try_get("last_seen_at")?,
        ban_reason: if staff && banned_until.is_some() {
            r.try_get("ban_reason")?
        } else {
            None
        },
        banned_until,
        is_active,
        stats,
        has_blog: false,
    })
}

pub struct MemberQuery {
    pub search: Option<String>,
    pub role: Option<Role>,
    pub online_only: Option<Vec<Uuid>>,
    pub sort: MemberSort,
    pub page: i64,
    pub per_page: i64,
}

pub async fn members(db: &PgPool, q: MemberQuery) -> Result<(Vec<MemberItem>, i64)> {
    let order = match q.sort {
        MemberSort::Newest => "u.created_at DESC",
        MemberSort::Name => "lower(u.display_name) ASC",
        MemberSort::Active => "u.last_seen_at DESC NULLS LAST",
        MemberSort::Posts => "post_count DESC, u.created_at",
    };
    let search = q.search.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let filter = "u.is_active \
        AND ($1::text IS NULL OR position($1 in lower(u.username::text)) > 0 \
             OR position($1 in lower(u.display_name)) > 0) \
        AND ($2::text IS NULL OR u.role = $2) \
        AND ($3::uuid[] IS NULL OR u.id = ANY($3))";
    let rows = sqlx::query(&format!(
        "SELECT {}, u.created_at, u.last_seen_at, \
           (SELECT count(*) FROM forum_posts p WHERE p.author_id = u.id AND NOT p.is_deleted) AS post_count \
         FROM users u WHERE {filter} ORDER BY {order} LIMIT $4 OFFSET $5",
        user_cols("u", "u")
    ))
    .bind(&search)
    .bind(q.role.map(|r| r.as_str()))
    .bind(&q.online_only)
    .bind(q.per_page)
    .bind(q.page.max(0) * q.per_page)
    .fetch_all(db)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM users u WHERE {filter}"))
        .bind(&search)
        .bind(q.role.map(|r| r.as_str()))
        .bind(&q.online_only)
        .fetch_one(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(MemberItem {
            user: user_from(&r, "u")?,
            created_at: r.try_get("created_at")?,
            last_seen_at: r.try_get("last_seen_at")?,
            post_count: r.try_get("post_count")?,
        });
    }
    Ok((out, total))
}

// ---------------------------------------------------------------------------
// Own settings
// ---------------------------------------------------------------------------

pub async fn settings(db: &PgPool, user_id: Uuid) -> Result<SettingsView> {
    let r = sqlx::query(
        "SELECT username::text AS username, email::text AS email, display_name, bio_md, \
                coalesce(location, '') AS location, coalesce(website, '') AS website, \
                theme, notify_prefs, avatar_file_id \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Benutzer"))?;
    let theme: String = r.try_get("theme")?;
    let prefs: serde_json::Value = r.try_get("notify_prefs")?;
    Ok(SettingsView {
        username: r.try_get("username")?,
        email: r.try_get("email")?,
        display_name: r.try_get("display_name")?,
        bio_md: r.try_get("bio_md")?,
        location: r.try_get("location")?,
        website: r.try_get("website")?,
        theme: Theme::parse(&theme),
        notify_prefs: serde_json::from_value(prefs).unwrap_or_default(),
        avatar_url: avatar_url(r.try_get("avatar_file_id")?),
    })
}

pub fn validate_display_name(name: &str) -> Result<String> {
    let name = name.trim();
    let len = name.chars().count();
    if len == 0 || len > 50 {
        return Err(Error::invalid("Der Anzeigename muss 1–50 Zeichen lang sein."));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(Error::invalid("Der Anzeigename enthält ungültige Zeichen."));
    }
    Ok(name.to_string())
}

pub async fn update_profile(db: &PgPool, user_id: Uuid, input: ProfileUpdate) -> Result<()> {
    let display_name = validate_display_name(&input.display_name)?;
    if input.bio_md.chars().count() > 5000 {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 5000 Zeichen)."));
    }
    let location = input.location.trim();
    if location.chars().count() > 100 {
        return Err(Error::invalid("Der Ort ist zu lang."));
    }
    let website = input.website.trim();
    if !website.is_empty()
        && (!(website.starts_with("https://") || website.starts_with("http://"))
            || website.len() > 200
            || website.contains(char::is_whitespace))
    {
        return Err(Error::invalid(
            "Die Website muss mit http:// oder https:// beginnen.",
        ));
    }
    let bio_html = markdown::render_mentions(db, &input.bio_md).await?.html;
    sqlx::query(
        "UPDATE users SET display_name = $2, bio_md = $3, bio_html = $4, \
            location = NULLIF($5, ''), website = NULLIF($6, ''), updated_at = now() \
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(display_name)
    .bind(&input.bio_md)
    .bind(bio_html)
    .bind(location)
    .bind(website)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_theme(db: &PgPool, user_id: Uuid, theme: Theme) -> Result<()> {
    sqlx::query("UPDATE users SET theme = $2 WHERE id = $1")
        .bind(user_id)
        .bind(theme.as_str())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn update_notify_prefs(db: &PgPool, user_id: Uuid, prefs: &NotifyPrefs) -> Result<()> {
    sqlx::query("UPDATE users SET notify_prefs = $2 WHERE id = $1")
        .bind(user_id)
        .bind(serde_json::to_value(prefs).map_err(Error::internal)?)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_avatar(db: &PgPool, user_id: Uuid, file_id: Option<Uuid>) -> Result<()> {
    sqlx::query("UPDATE users SET avatar_file_id = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(file_id)
        .execute(db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Administration
// ---------------------------------------------------------------------------

pub async fn admin_list(
    db: &PgPool,
    search: Option<String>,
    filter: &str,
    page: i64,
    per_page: i64,
) -> Result<(Vec<AdminUserRow>, i64)> {
    let search = search.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let cond = match filter {
        "banned" => "u.banned_until IS NOT NULL AND u.banned_until > now()",
        "inactive" => "NOT u.is_active",
        "staff" => "u.role IN ('admin', 'moderator')",
        _ => "TRUE",
    };
    let where_sql = format!(
        "{cond} AND ($1::text IS NULL OR position($1 in lower(u.username::text)) > 0 \
         OR position($1 in lower(u.display_name)) > 0 OR position($1 in lower(u.email::text)) > 0)"
    );
    let rows = sqlx::query(&format!(
        "SELECT u.id, u.username::text AS username, u.display_name, u.email::text AS email, u.role, \
                u.is_active, u.banned_until, u.ban_reason, u.created_at, u.last_seen_at, \
                {storage} AS storage_used \
         FROM users u WHERE {where_sql} ORDER BY u.created_at DESC LIMIT $2 OFFSET $3",
        storage = crate::storage_usage_sql("u.id"),
    ))
    .bind(&search)
    .bind(per_page)
    .bind(page.max(0) * per_page)
    .fetch_all(db)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM users u WHERE {where_sql}"))
        .bind(&search)
        .fetch_one(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let role: String = r.try_get("role")?;
        out.push(AdminUserRow {
            id: r.try_get("id")?,
            username: r.try_get("username")?,
            display_name: r.try_get("display_name")?,
            email: r.try_get("email")?,
            role: Role::parse(&role).unwrap_or_default(),
            is_active: r.try_get("is_active")?,
            banned_until: r.try_get("banned_until")?,
            ban_reason: r.try_get("ban_reason")?,
            created_at: r.try_get("created_at")?,
            last_seen_at: r.try_get("last_seen_at")?,
            storage_used: r.try_get("storage_used")?,
        });
    }
    Ok((out, total))
}

async fn target_role(db: &PgPool, user_id: Uuid) -> Result<Role> {
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    role.and_then(|r| Role::parse(&r)).ok_or(Error::NotFound("Benutzer"))
}

pub async fn set_role(state: &AppState, actor: &AuthUser, user_id: Uuid, role: Role) -> Result<()> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    let current = target_role(&state.db, user_id).await?;
    if current == Role::Admin && role != Role::Admin {
        let admins: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'admin' AND is_active")
            .fetch_one(&state.db)
            .await?;
        if admins <= 1 {
            return Err(Error::invalid("Der letzte Admin kann nicht herabgestuft werden."));
        }
    }
    sqlx::query("UPDATE users SET role = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(role.as_str())
        .execute(&state.db)
        .await?;
    modlog::record(
        &state.db,
        Some(actor.id),
        "role",
        Some((SubjectType::User, user_id)),
        Some(user_id),
        &format!("{} → {}", current.label(), role.label()),
    )
    .await
}

/// Bans a user for `days` days, or permanently when `days` is `None`.
pub async fn ban(
    state: &AppState,
    actor: &AuthUser,
    user_id: Uuid,
    days: Option<i64>,
    reason: &str,
) -> Result<()> {
    if !actor.is_mod() {
        return Err(Error::Forbidden);
    }
    if actor.id == user_id {
        return Err(Error::invalid("Du kannst dich nicht selbst sperren."));
    }
    let target = target_role(&state.db, user_id).await?;
    if target.at_least(Role::Moderator) && !actor.is_admin() {
        return Err(Error::invalid("Nur Admins können Moderatoren sperren."));
    }
    if target == Role::Admin {
        return Err(Error::invalid("Admins können nicht gesperrt werden."));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(Error::invalid("Bitte gib einen Grund an."));
    }
    let until = match days {
        Some(d) if d > 0 => Utc::now() + Duration::days(d.min(36500)),
        _ => DateTime::<Utc>::from_timestamp(253_402_214_400, 0).unwrap_or_else(Utc::now),
    };
    sqlx::query("UPDATE users SET banned_until = $2, ban_reason = $3 WHERE id = $1")
        .bind(user_id)
        .bind(until)
        .bind(reason)
        .execute(&state.db)
        .await?;
    state.hub.send_user(
        user_id,
        &shared::ws::ServerMsg::Error {
            message: "Dein Konto wurde gesperrt.".into(),
        },
    );
    let detail = match days {
        Some(d) if d > 0 => format!("{d} Tage: {reason}"),
        _ => format!("dauerhaft: {reason}"),
    };
    modlog::record(&state.db, Some(actor.id), "ban", Some((SubjectType::User, user_id)), Some(user_id), &detail).await
}

pub async fn unban(state: &AppState, actor: &AuthUser, user_id: Uuid) -> Result<()> {
    if !actor.is_mod() {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE users SET banned_until = NULL, ban_reason = NULL WHERE id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await?;
    modlog::record(&state.db, Some(actor.id), "unban", Some((SubjectType::User, user_id)), Some(user_id), "").await
}

pub async fn set_active(state: &AppState, actor: &AuthUser, user_id: Uuid, active: bool) -> Result<()> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    if actor.id == user_id {
        return Err(Error::invalid("Du kannst dein eigenes Konto nicht deaktivieren."));
    }
    sqlx::query("UPDATE users SET is_active = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(active)
        .execute(&state.db)
        .await?;
    modlog::record(
        &state.db,
        Some(actor.id),
        if active { "activate" } else { "deactivate" },
        Some((SubjectType::User, user_id)),
        Some(user_id),
        "",
    )
    .await
}

pub async fn set_quota(state: &AppState, actor: &AuthUser, user_id: Uuid, quota_mb: Option<i64>) -> Result<()> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE users SET storage_quota_bytes = $2 WHERE id = $1")
        .bind(user_id)
        .bind(quota_mb.map(|mb| mb.max(0) * 1024 * 1024))
        .execute(&state.db)
        .await?;
    Ok(())
}

/// Recent members for the start page.
pub async fn newest(db: &PgPool, limit: i64) -> Result<Vec<UserSummary>> {
    let rows = sqlx::query(&format!(
        "SELECT {} FROM users u WHERE u.is_active ORDER BY u.created_at DESC LIMIT $1",
        user_cols("u", "u")
    ))
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(|r| user_from(r, "u")).collect::<Result<_, _>>()?)
}

pub fn page_of<T>(items: Vec<T>, next: Option<String>) -> Page<T> {
    Page::new(items, next)
}
