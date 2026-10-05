use shared::AuthUser;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
    display_name: String,
    role: String,
}

impl From<UserRow> for AuthUser {
    fn from(r: UserRow) -> Self {
        AuthUser {
            id: r.id,
            username: r.username,
            display_name: r.display_name,
            role: r.role,
        }
    }
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<AuthUser>, sqlx::Error> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, display_name, role FROM users WHERE id = $1 AND is_active",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

/// Returns `(user_id, password_hash)` for a username-or-email lookup used at login.
pub async fn find_credentials(
    pool: &PgPool,
    username_or_email: &str,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, password_hash FROM users WHERE (username = $1 OR email = $1) AND is_active",
    )
    .bind(username_or_email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn is_first_user(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(count == 0)
}

pub async fn create_user(
    pool: &PgPool,
    username: &str,
    email: &str,
    password_hash: &str,
    display_name: &str,
    role: &str,
) -> Result<AuthUser, sqlx::Error> {
    let id = Uuid::now_v7();
    let row = sqlx::query_as::<_, UserRow>(
        r#"INSERT INTO users (id, username, email, password_hash, display_name, role)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, username, display_name, role"#,
    )
    .bind(id)
    .bind(username)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .bind(role)
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}
