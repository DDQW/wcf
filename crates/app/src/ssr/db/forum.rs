use chrono::{DateTime, Utc};
use shared::{CategorySummary, PostView, TopicSummary};
use sqlx::PgPool;
use uuid::Uuid;

use crate::ssr::markdown;
use shared::DEFAULT_PAGE_SIZE;

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: Uuid,
    slug: String,
    name: String,
    description: Option<String>,
}

impl From<CategoryRow> for CategorySummary {
    fn from(r: CategoryRow) -> Self {
        CategorySummary {
            id: r.id,
            slug: r.slug,
            name: r.name,
            description: r.description,
        }
    }
}

pub async fn list_categories(pool: &PgPool) -> Result<Vec<CategorySummary>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CategoryRow>(
        "SELECT id, slug, name, description FROM forum_categories ORDER BY position, name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn create_category(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
) -> Result<CategorySummary, sqlx::Error> {
    let id = Uuid::now_v7();
    let slug = markdown::slugify(name, &id);
    let row = sqlx::query_as::<_, CategoryRow>(
        r#"INSERT INTO forum_categories (id, slug, name, description)
           VALUES ($1, $2, $3, $4)
           RETURNING id, slug, name, description"#,
    )
    .bind(id)
    .bind(&slug)
    .bind(name)
    .bind(description)
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}

#[derive(sqlx::FromRow)]
struct TopicRow {
    id: Uuid,
    category_id: Uuid,
    title: String,
    slug: String,
    author_display_name: String,
    is_pinned: bool,
    is_locked: bool,
    reply_count: i32,
    last_post_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl From<TopicRow> for TopicSummary {
    fn from(r: TopicRow) -> Self {
        TopicSummary {
            id: r.id,
            category_id: r.category_id,
            title: r.title,
            slug: r.slug,
            author_display_name: r.author_display_name,
            is_pinned: r.is_pinned,
            is_locked: r.is_locked,
            reply_count: r.reply_count,
            last_post_at: r.last_post_at,
            created_at: r.created_at,
        }
    }
}

const TOPIC_SELECT: &str = r#"
    SELECT t.id, t.category_id, t.title, t.slug, u.display_name AS author_display_name,
           t.is_pinned, t.is_locked, t.reply_count, t.last_post_at, t.created_at
    FROM forum_topics t
    JOIN users u ON u.id = t.author_id
"#;

/// Keyset-paginated topic list for a category, newest activity first.
/// `cursor` is `(last_post_at, id)` of the last row the caller already has.
pub async fn list_topics(
    pool: &PgPool,
    category_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: Option<i64>,
) -> Result<Vec<TopicSummary>, sqlx::Error> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let rows = if let Some((ts, id)) = cursor {
        let sql = format!(
            "{TOPIC_SELECT} WHERE t.category_id = $1 AND (t.last_post_at, t.id) < ($2, $3) \
             ORDER BY t.last_post_at DESC, t.id DESC LIMIT $4"
        );
        sqlx::query_as::<_, TopicRow>(&sql)
            .bind(category_id)
            .bind(ts)
            .bind(id)
            .bind(limit)
            .fetch_all(pool)
            .await?
    } else {
        let sql = format!(
            "{TOPIC_SELECT} WHERE t.category_id = $1 ORDER BY t.last_post_at DESC, t.id DESC LIMIT $2"
        );
        sqlx::query_as::<_, TopicRow>(&sql)
            .bind(category_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
    };
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Creates a topic plus its opening (root) post in one transaction.
pub async fn get_topic(pool: &PgPool, topic_id: Uuid) -> Result<Option<TopicSummary>, sqlx::Error> {
    let sql = format!("{TOPIC_SELECT} WHERE t.id = $1");
    let row = sqlx::query_as::<_, TopicRow>(&sql)
        .bind(topic_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(Into::into))
}

pub async fn create_topic(
    pool: &PgPool,
    category_id: Uuid,
    author_id: Uuid,
    title: &str,
    body_md: &str,
) -> Result<Uuid, sqlx::Error> {
    let topic_id = Uuid::now_v7();
    let root_post_id = Uuid::now_v7();
    let slug = markdown::slugify(title, &topic_id);
    let body_html = markdown::render(body_md);

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO forum_topics (id, category_id, author_id, title, slug) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(topic_id)
    .bind(category_id)
    .bind(author_id)
    .bind(title)
    .bind(&slug)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO forum_posts (id, topic_id, author_id, parent_post_id, thread_root_id, depth, body_md, body_html)
           VALUES ($1, $2, $3, NULL, $1, 0, $4, $5)"#,
    )
    .bind(root_post_id)
    .bind(topic_id)
    .bind(author_id)
    .bind(body_md)
    .bind(&body_html)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(topic_id)
}

#[derive(sqlx::FromRow)]
struct PostRow {
    id: Uuid,
    topic_id: Uuid,
    author_id: Uuid,
    author_display_name: String,
    parent_post_id: Option<Uuid>,
    thread_root_id: Uuid,
    depth: i16,
    body_html: String,
    created_at: DateTime<Utc>,
}

impl From<PostRow> for PostView {
    fn from(r: PostRow) -> Self {
        PostView {
            id: r.id,
            topic_id: r.topic_id,
            author_id: r.author_id,
            author_display_name: r.author_display_name,
            parent_post_id: r.parent_post_id,
            thread_root_id: r.thread_root_id,
            depth: r.depth,
            body_html: r.body_html,
            created_at: r.created_at,
        }
    }
}

const POST_SELECT: &str = r#"
    SELECT p.id, p.topic_id, p.author_id, u.display_name AS author_display_name,
           p.parent_post_id, p.thread_root_id, p.depth, p.body_html, p.created_at
    FROM forum_posts p
    JOIN users u ON u.id = p.author_id
    WHERE p.topic_id = $1 AND p.is_deleted = FALSE
"#;

/// Keyset-paginated, oldest-first post list for a topic. The client groups
/// the flat result by `thread_root_id` into collapsible blocks.
pub async fn list_posts(
    pool: &PgPool,
    topic_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: Option<i64>,
) -> Result<Vec<PostView>, sqlx::Error> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let rows = if let Some((ts, id)) = cursor {
        let sql =
            format!("{POST_SELECT} AND (p.created_at, p.id) > ($2, $3) ORDER BY p.created_at ASC, p.id ASC LIMIT $4");
        sqlx::query_as::<_, PostRow>(&sql)
            .bind(topic_id)
            .bind(ts)
            .bind(id)
            .bind(limit)
            .fetch_all(pool)
            .await?
    } else {
        let sql = format!("{POST_SELECT} ORDER BY p.created_at ASC, p.id ASC LIMIT $2");
        sqlx::query_as::<_, PostRow>(&sql)
            .bind(topic_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
    };
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(Debug, thiserror::Error)]
pub enum CreatePostError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("parent post not found in this topic")]
    ParentNotInTopic,
}

pub async fn create_post(
    pool: &PgPool,
    topic_id: Uuid,
    author_id: Uuid,
    parent_post_id: Option<Uuid>,
    body_md: &str,
) -> Result<Uuid, CreatePostError> {
    let post_id = Uuid::now_v7();
    let body_html = markdown::render(body_md);

    let (thread_root_id, depth): (Uuid, i16) = if let Some(parent_id) = parent_post_id {
        let parent: Option<(Uuid, i16)> = sqlx::query_as(
            "SELECT thread_root_id, depth FROM forum_posts WHERE id = $1 AND topic_id = $2",
        )
        .bind(parent_id)
        .bind(topic_id)
        .fetch_optional(pool)
        .await?;
        let (root, parent_depth) = parent.ok_or(CreatePostError::ParentNotInTopic)?;
        (root, parent_depth + 1)
    } else {
        (post_id, 0)
    };

    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"INSERT INTO forum_posts (id, topic_id, author_id, parent_post_id, thread_root_id, depth, body_md, body_html)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
    )
    .bind(post_id)
    .bind(topic_id)
    .bind(author_id)
    .bind(parent_post_id)
    .bind(thread_root_id)
    .bind(depth)
    .bind(body_md)
    .bind(&body_html)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE forum_topics SET reply_count = reply_count + 1, last_post_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(post_id)
}
