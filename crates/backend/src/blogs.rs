//! Blogs: one blog per user, posts with drafts/scheduling, subscriptions
//! and RSS feeds.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use shared::blog::{
    BlogFilter, BlogPostDetail, BlogPostEdit, BlogPostItem, BlogPostPage, BlogView, PostNeighbour,
    PostStatus, SavedPost, POSTS_PER_PAGE,
};
use shared::files::FilePurpose;
use shared::forum::{normalize_tags, TagCount};
use shared::{AuthUser, Cursor, NotificationKind, SubjectType};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::users::{user_cols, user_from};
use crate::{files, markdown, modlog, reactions};

// ---------------------------------------------------------------------------
// Blogs
// ---------------------------------------------------------------------------

const BLOG_SELECT: &str = "
    SELECT b.id, b.title, b.description, b.created_at,
           (SELECT count(*) FROM blog_posts p WHERE p.blog_id = b.id AND p.status = 'published' AND p.published_at <= now()) AS post_count,
           (SELECT max(p.published_at) FROM blog_posts p WHERE p.blog_id = b.id AND p.status = 'published' AND p.published_at <= now()) AS last_post_at,
           (SELECT count(*) FROM blog_subscriptions s WHERE s.blog_id = b.id) AS subscriber_count,
           EXISTS (SELECT 1 FROM blog_subscriptions s WHERE s.blog_id = b.id AND s.user_id = $1) AS subscribed,
           u.id AS owner_id, u.username::text AS owner_username, u.display_name AS owner_display_name,
           u.avatar_file_id AS owner_avatar, u.role AS owner_role
    FROM blogs b JOIN users u ON u.id = b.owner_id
";

fn blog_from(r: &PgRow) -> Result<BlogView> {
    Ok(BlogView {
        id: r.try_get("id")?,
        owner: user_from(r, "owner")?,
        title: r.try_get("title")?,
        description: r.try_get("description")?,
        post_count: r.try_get("post_count")?,
        subscriber_count: r.try_get("subscriber_count")?,
        subscribed: r.try_get("subscribed")?,
        created_at: r.try_get("created_at")?,
        last_post_at: r.try_get("last_post_at")?,
    })
}

pub async fn blog_by_username(db: &PgPool, viewer: Option<&AuthUser>, username: &str) -> Result<BlogView> {
    let row = sqlx::query(&format!("{BLOG_SELECT} WHERE u.username = $2::citext AND u.is_active"))
        .bind(viewer.map(|v| v.id))
        .bind(username)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Blog"))?;
    blog_from(&row)
}

pub async fn blog_of(db: &PgPool, viewer: Option<&AuthUser>, owner: uuid::Uuid) -> Result<Option<BlogView>> {
    let row = sqlx::query(&format!("{BLOG_SELECT} WHERE b.owner_id = $2"))
        .bind(viewer.map(|v| v.id))
        .bind(owner)
        .fetch_optional(db)
        .await?;
    row.map(|r| blog_from(&r)).transpose()
}

pub async fn all_blogs(db: &PgPool, viewer: Option<&AuthUser>) -> Result<Vec<BlogView>> {
    let rows = sqlx::query(&format!(
        "{BLOG_SELECT} WHERE u.is_active ORDER BY last_post_at DESC NULLS LAST, b.created_at DESC LIMIT 200"
    ))
    .bind(viewer.map(|v| v.id))
    .fetch_all(db)
    .await?;
    rows.iter().map(blog_from).collect()
}

/// Creates the user's blog or updates its title/description.
pub async fn save_blog(db: &PgPool, user: &AuthUser, title: &str, description: &str) -> Result<BlogView> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 100 {
        return Err(Error::invalid("Der Blogtitel muss 1–100 Zeichen lang sein."));
    }
    if description.chars().count() > 1000 {
        return Err(Error::invalid("Die Beschreibung ist zu lang."));
    }
    sqlx::query(
        "INSERT INTO blogs (id, owner_id, title, description) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (owner_id) DO UPDATE SET title = EXCLUDED.title, description = EXCLUDED.description, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(title)
    .bind(description.trim())
    .execute(db)
    .await?;
    blog_of(db, Some(user), user.id)
        .await?
        .ok_or(Error::NotFound("Blog"))
}

pub async fn set_subscription(state: &AppState, user: &AuthUser, blog_id: Uuid, on: bool) -> Result<()> {
    if on {
        sqlx::query("INSERT INTO blog_subscriptions (user_id, blog_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(blog_id)
            .execute(&state.db)
            .await?;
    } else {
        sqlx::query("DELETE FROM blog_subscriptions WHERE user_id = $1 AND blog_id = $2")
            .bind(user.id)
            .bind(blog_id)
            .execute(&state.db)
            .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Posts
// ---------------------------------------------------------------------------

fn post_cols() -> String {
    format!(
        "p.id, p.blog_id, b.title AS blog_title, p.title, p.slug, p.excerpt, p.cover_file_id, p.tags,
         p.status, p.published_at, p.created_at, p.updated_at, p.reading_minutes, p.view_count,
         (SELECT count(*) FROM comments c WHERE c.subject_type = 'blog_post' AND c.subject_id = p.id AND c.deleted_at IS NULL) AS comment_count,
         (SELECT count(*) FROM reactions r WHERE r.subject_type = 'blog_post' AND r.subject_id = p.id) AS like_count,
         {}",
        user_cols("u", "author")
    )
}

const POST_FROM: &str = " FROM blog_posts p JOIN blogs b ON b.id = p.blog_id JOIN users u ON u.id = p.author_id ";

fn post_select() -> String {
    format!("SELECT {}{POST_FROM}", post_cols())
}

const PUBLISHED: &str = "p.status = 'published' AND p.published_at <= now()";

fn status_of(status: &str, published_at: Option<DateTime<Utc>>) -> PostStatus {
    match (status, published_at) {
        ("published", Some(t)) if t > Utc::now() => PostStatus::Scheduled,
        ("published", _) => PostStatus::Published,
        _ => PostStatus::Draft,
    }
}

fn post_from(r: &PgRow) -> Result<BlogPostItem> {
    let status: String = r.try_get("status")?;
    let published_at: Option<DateTime<Utc>> = r.try_get("published_at")?;
    let cover: Option<Uuid> = r.try_get("cover_file_id")?;
    Ok(BlogPostItem {
        id: r.try_get("id")?,
        blog_id: r.try_get("blog_id")?,
        blog_title: r.try_get("blog_title")?,
        author: user_from(r, "author")?,
        title: r.try_get("title")?,
        slug: r.try_get("slug")?,
        excerpt: r.try_get("excerpt")?,
        cover_url: cover.map(|id| format!("/files/{id}/preview")),
        tags: r.try_get("tags")?,
        status: status_of(&status, published_at),
        published_at,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        reading_minutes: r.try_get("reading_minutes")?,
        view_count: r.try_get("view_count")?,
        comment_count: r.try_get("comment_count")?,
        like_count: r.try_get("like_count")?,
    })
}

pub async fn list_posts(db: &PgPool, viewer: Option<&AuthUser>, filter: BlogFilter, cursor: Option<String>, limit: Option<i64>) -> Result<BlogPostPage> {
    let limit = limit.unwrap_or(POSTS_PER_PAGE).clamp(1, 50);
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(post_select());
    qb.push(format!(" WHERE {PUBLISHED} AND u.is_active"));
    match &filter {
        BlogFilter::All => {}
        BlogFilter::Blog(id) => {
            qb.push(" AND p.blog_id = ");
            qb.push_bind(*id);
        }
        BlogFilter::Tag(tag) => {
            qb.push(" AND ");
            qb.push_bind(tag.clone());
            qb.push(" = ANY(p.tags)");
        }
        BlogFilter::Following => {
            let Some(v) = viewer else {
                return Ok(BlogPostPage { items: Vec::new(), next_cursor: None });
            };
            qb.push(" AND p.blog_id IN (SELECT blog_id FROM blog_subscriptions WHERE user_id = ");
            qb.push_bind(v.id);
            qb.push(")");
        }
    }
    if let Some(c) = cursor.as_deref().and_then(|c| Cursor::decode(c).ok()) {
        qb.push(" AND (p.published_at, p.id) < (");
        qb.push_bind(c.ts);
        qb.push(", ");
        qb.push_bind(c.id);
        qb.push(")");
    }
    qb.push(" ORDER BY p.published_at DESC, p.id DESC LIMIT ");
    qb.push_bind(limit + 1);
    let rows = qb.build().fetch_all(db).await?;
    let mut items: Vec<BlogPostItem> = rows.iter().map(post_from).collect::<Result<_>>()?;
    let next_cursor = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items
            .last()
            .and_then(|p| p.published_at.map(|t| Cursor::new(t, p.id).encode()))
    } else {
        None
    };
    Ok(BlogPostPage { items, next_cursor })
}

/// All posts of the user's own blog, including drafts and scheduled ones.
pub async fn my_posts(db: &PgPool, user: &AuthUser) -> Result<Vec<BlogPostItem>> {
    let rows = sqlx::query(&format!(
        "{} WHERE b.owner_id = $1 ORDER BY coalesce(p.published_at, p.updated_at) DESC",
        post_select()
    ))
    .bind(user.id)
    .fetch_all(db)
    .await?;
    rows.iter().map(post_from).collect()
}

pub async fn popular_tags(db: &PgPool, limit: i64) -> Result<Vec<TagCount>> {
    let rows = sqlx::query(&format!(
        "SELECT tag, count(*) AS n FROM blog_posts p, unnest(p.tags) tag WHERE {PUBLISHED} \
         GROUP BY tag ORDER BY n DESC, tag LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|r| Ok(TagCount { tag: r.try_get("tag")?, count: r.try_get("n")? }))
        .collect()
}

async fn neighbour(db: &PgPool, blog_id: Uuid, published_at: DateTime<Utc>, id: Uuid, newer: bool) -> Result<Option<PostNeighbour>> {
    let (op, order) = if newer { (">", "ASC") } else { ("<", "DESC") };
    let row = sqlx::query(&format!(
        "SELECT p.title, p.slug, u.username::text AS username FROM blog_posts p JOIN users u ON u.id = p.author_id \
         WHERE p.blog_id = $1 AND {PUBLISHED} AND (p.published_at, p.id) {op} ($2, $3) \
         ORDER BY p.published_at {order}, p.id {order} LIMIT 1"
    ))
    .bind(blog_id)
    .bind(published_at)
    .bind(id)
    .fetch_optional(db)
    .await?;
    row
        .map(|r| -> Result<PostNeighbour> {
            let username: String = r.try_get("username")?;
            let slug: String = r.try_get("slug")?;
            Ok(PostNeighbour {
                title: r.try_get("title")?,
                url: format!("/blog/{username}/{slug}"),
            })
        })
        .transpose()
}

pub async fn get_post(state: &AppState, viewer: Option<&AuthUser>, username: &str, slug: &str) -> Result<BlogPostDetail> {
    let db = &state.db;
    let row = sqlx::query(&format!(
        "SELECT {}, p.body_html, p.allow_comments, b.owner_id{POST_FROM} \
         JOIN users o ON o.id = b.owner_id WHERE o.username = $1::citext AND p.slug = $2",
        post_cols()
    ))
    .bind(username)
    .bind(slug)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Beitrag"))?;
    let post = post_from(&row)?;
    let owner_id: Uuid = row.try_get("owner_id")?;
    let is_owner = viewer.is_some_and(|v| v.id == owner_id);
    let staff = viewer.is_some_and(|v| v.is_mod());
    if post.status != PostStatus::Published && !is_owner && !staff {
        return Err(Error::NotFound("Beitrag"));
    }
    if !is_owner && post.status == PostStatus::Published {
        sqlx::query("UPDATE blog_posts SET view_count = view_count + 1 WHERE id = $1")
            .bind(post.id)
            .execute(db)
            .await?;
    }
    let blog = blog_of(db, viewer, owner_id).await?.ok_or(Error::NotFound("Blog"))?;
    let (newer, older) = match post.published_at {
        Some(t) if post.status == PostStatus::Published => (
            neighbour(db, post.blog_id, t, post.id, true).await?,
            neighbour(db, post.blog_id, t, post.id, false).await?,
        ),
        _ => (None, None),
    };
    let reactions = reactions::counts_one(db, SubjectType::BlogPost, post.id, viewer.map(|v| v.id)).await?;
    Ok(BlogPostDetail {
        body_html: row.try_get("body_html")?,
        allow_comments: row.try_get("allow_comments")?,
        can_edit: is_owner || staff,
        reactions,
        newer,
        older,
        blog,
        post,
    })
}

pub async fn post_for_edit(db: &PgPool, user: &AuthUser, id: Uuid) -> Result<BlogPostEdit> {
    let row = sqlx::query(
        "SELECT p.id, p.title, p.slug, p.excerpt, p.body_md, p.cover_file_id, p.tags, p.status, p.published_at, \
                p.allow_comments, b.owner_id FROM blog_posts p JOIN blogs b ON b.id = p.blog_id WHERE p.id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Beitrag"))?;
    let owner: Uuid = row.try_get("owner_id")?;
    if owner != user.id && !user.is_mod() {
        return Err(Error::Forbidden);
    }
    let status: String = row.try_get("status")?;
    let cover: Option<Uuid> = row.try_get("cover_file_id")?;
    Ok(BlogPostEdit {
        id: Some(id),
        title: row.try_get("title")?,
        slug: row.try_get("slug")?,
        excerpt: row.try_get("excerpt")?,
        body_md: row.try_get("body_md")?,
        cover_file_id: cover,
        cover_url: cover.map(|c| format!("/files/{c}/preview")),
        tags: row.try_get("tags")?,
        publish: status == "published",
        publish_at: row.try_get("published_at")?,
        allow_comments: row.try_get("allow_comments")?,
    })
}

async fn unique_slug(db: &PgPool, blog_id: Uuid, wanted: &str, exclude: Option<Uuid>) -> Result<String> {
    let base = {
        let b = shared::util::slug_base(wanted);
        if b.is_empty() { "beitrag".to_string() } else { b }
    };
    let mut candidate = base.clone();
    for i in 2..200 {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM blog_posts WHERE blog_id = $1 AND slug = $2 AND ($3::uuid IS NULL OR id <> $3))",
        )
        .bind(blog_id)
        .bind(&candidate)
        .bind(exclude)
        .fetch_one(db)
        .await?;
        if !taken {
            return Ok(candidate);
        }
        candidate = format!("{base}-{i}");
    }
    Ok(format!("{base}-{}", &Uuid::now_v7().simple().to_string()[..6]))
}

pub async fn save_post(state: &AppState, user: &AuthUser, input: BlogPostEdit) -> Result<SavedPost> {
    if !state.settings().modules.blogs {
        return Err(Error::invalid("Blogs sind deaktiviert."));
    }
    state
        .limiter
        .hit(&format!("blogpost:{}", user.id), 30, StdDuration::from_secs(3600))?;
    let title = input.title.trim();
    if title.chars().count() < 2 || title.chars().count() > 200 {
        return Err(Error::invalid("Der Titel muss 2–200 Zeichen lang sein."));
    }
    if input.body_md.trim().is_empty() {
        return Err(Error::invalid("Der Beitrag ist leer."));
    }
    if input.body_md.chars().count() > 200_000 {
        return Err(Error::invalid("Der Beitrag ist zu lang."));
    }
    if input.excerpt.chars().count() > 500 {
        return Err(Error::invalid("Der Auszug ist zu lang (max. 500 Zeichen)."));
    }
    if let Some(cover) = input.cover_file_id {
        let rec = files::get_record(&state.db, cover).await?.ok_or(Error::NotFound("Titelbild"))?;
        if rec.owner_id != user.id || !rec.view.mime_type.starts_with("image/") {
            return Err(Error::invalid("Ungültiges Titelbild."));
        }
        if rec.view.purpose != FilePurpose::Blog {
            sqlx::query("UPDATE files SET visibility = 'public' WHERE id = $1")
                .bind(cover)
                .execute(&state.db)
                .await?;
        }
    }

    // The editor works on the author's own blog (created on first save).
    let (blog_id, owner_id) = match input.id {
        Some(id) => {
            let r: Option<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT p.blog_id, b.owner_id FROM blog_posts p JOIN blogs b ON b.id = p.blog_id WHERE p.id = $1",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
            let (blog_id, owner_id) = r.ok_or(Error::NotFound("Beitrag"))?;
            if owner_id != user.id && !user.is_mod() {
                return Err(Error::Forbidden);
            }
            (blog_id, owner_id)
        }
        None => {
            let blog = match blog_of(&state.db, Some(user), user.id).await? {
                Some(b) => b,
                None => save_blog(&state.db, user, &format!("Blog von {}", user.display_name), "").await?,
            };
            (blog.id, user.id)
        }
    };

    let slug_wanted = if input.slug.trim().is_empty() { title } else { input.slug.trim() };
    let slug = unique_slug(&state.db, blog_id, slug_wanted, input.id).await?;
    let rendered = markdown::render_mentions(&state.db, &input.body_md).await?;
    let excerpt = if input.excerpt.trim().is_empty() {
        markdown::excerpt(&input.body_md, 280)
    } else {
        input.excerpt.trim().to_string()
    };
    let tags = normalize_tags(&input.tags);
    let minutes = shared::util::reading_time_minutes(&markdown::plain_text(&input.body_md)) as i32;
    let status = if input.publish { "published" } else { "draft" };
    let published_at = if input.publish {
        Some(input.publish_at.unwrap_or_else(Utc::now))
    } else {
        input.publish_at
    };

    let id = match input.id {
        Some(id) => {
            sqlx::query(
                "UPDATE blog_posts SET title = $2, slug = $3, excerpt = $4, body_md = $5, body_html = $6, \
                    cover_file_id = $7, tags = $8, status = $9, published_at = $10, allow_comments = $11, \
                    reading_minutes = $12, updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .bind(title)
            .bind(&slug)
            .bind(&excerpt)
            .bind(&input.body_md)
            .bind(&rendered.html)
            .bind(input.cover_file_id)
            .bind(&tags)
            .bind(status)
            .bind(published_at)
            .bind(input.allow_comments)
            .bind(minutes)
            .execute(&state.db)
            .await?;
            if owner_id != user.id {
                modlog::record(&state.db, Some(user.id), "edit_content", Some((SubjectType::BlogPost, id)), Some(owner_id), title).await?;
            }
            id
        }
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO blog_posts (id, blog_id, author_id, title, slug, excerpt, body_md, body_html, cover_file_id, \
                    tags, status, published_at, allow_comments, reading_minutes) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            )
            .bind(id)
            .bind(blog_id)
            .bind(owner_id)
            .bind(title)
            .bind(&slug)
            .bind(&excerpt)
            .bind(&input.body_md)
            .bind(&rendered.html)
            .bind(input.cover_file_id)
            .bind(&tags)
            .bind(status)
            .bind(published_at)
            .bind(input.allow_comments)
            .bind(minutes)
            .execute(&state.db)
            .await?;
            id
        }
    };
    // Immediately published posts notify right away; scheduled ones via job.
    publish_due(state).await?;
    let username: String = sqlx::query_scalar("SELECT username::text FROM users WHERE id = $1")
        .bind(owner_id)
        .fetch_one(&state.db)
        .await?;
    Ok(SavedPost {
        id,
        url: format!("/blog/{username}/{slug}"),
    })
}

pub async fn delete_post(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let r: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT b.owner_id, p.title FROM blog_posts p JOIN blogs b ON b.id = p.blog_id WHERE p.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (owner, title) = r.ok_or(Error::NotFound("Beitrag"))?;
    if owner != user.id && !user.is_mod() {
        return Err(Error::Forbidden);
    }
    sqlx::query("DELETE FROM blog_posts WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM comments WHERE subject_type = 'blog_post' AND subject_id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    reactions::delete_for(&state.db, SubjectType::BlogPost, &[id]).await?;
    if owner != user.id {
        modlog::record(&state.db, Some(user.id), "delete_content", Some((SubjectType::BlogPost, id)), Some(owner), &title).await?;
    }
    Ok(())
}

/// Notifies subscribers (and mentioned users) about posts that became
/// visible: immediately published ones and scheduled ones whose time came.
pub async fn publish_due(state: &AppState) -> Result<()> {
    let due: Vec<(Uuid, Uuid, Uuid, String, String, String, String)> = sqlx::query_as(
        "UPDATE blog_posts p SET notified = TRUE FROM blogs b, users u \
         WHERE b.id = p.blog_id AND u.id = p.author_id AND p.status = 'published' AND p.published_at <= now() \
           AND NOT p.notified \
         RETURNING p.id, p.blog_id, p.author_id, p.title, p.slug, u.username::text, p.body_md",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, blog_id, author, title, slug, username, body) in due {
        let link = format!("/blog/{username}/{slug}");
        let subs: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM blog_subscriptions WHERE blog_id = $1")
            .bind(blog_id)
            .fetch_all(&state.db)
            .await?;
        let author_name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(author)
            .fetch_one(&state.db)
            .await?;
        for uid in subs {
            notifications::notify(
                state,
                NewNotification {
                    user_id: uid,
                    kind: NotificationKind::Subscription,
                    actor_id: Some(author),
                    title: format!("Neuer Blogbeitrag von {author_name}: {title}"),
                    body: markdown::excerpt(&body, 160),
                    link: link.clone(),
                    subject: Some((SubjectType::BlogPost, id)),
                },
            )
            .await;
        }
        for uid in markdown::render_mentions(&state.db, &body).await?.mentioned {
            notifications::notify(
                state,
                NewNotification {
                    user_id: uid,
                    kind: NotificationKind::Mention,
                    actor_id: Some(author),
                    title: format!("{author_name} hat dich im Blogbeitrag „{title}“ erwähnt"),
                    body: markdown::excerpt(&body, 160),
                    link: link.clone(),
                    subject: Some((SubjectType::BlogPost, id)),
                },
            )
            .await;
        }
    }
    Ok(())
}

/// Subject info for reactions/comments/reports on blog posts.
pub async fn subject_info(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<Option<crate::subjects::SubjectInfo>> {
    let row = sqlx::query(
        "SELECT p.title, p.slug, p.status, p.published_at, p.allow_comments, b.owner_id, u.username::text AS username \
         FROM blog_posts p JOIN blogs b ON b.id = p.blog_id JOIN users u ON u.id = b.owner_id WHERE p.id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let status: String = r.try_get("status")?;
    let published_at: Option<DateTime<Utc>> = r.try_get("published_at")?;
    let owner: Uuid = r.try_get("owner_id")?;
    let visible = status_of(&status, published_at) == PostStatus::Published
        || viewer.is_some_and(|v| v.id == owner || v.is_mod());
    if !visible {
        return Ok(None);
    }
    let title: String = r.try_get("title")?;
    let username: String = r.try_get("username")?;
    let slug: String = r.try_get("slug")?;
    Ok(Some(crate::subjects::SubjectInfo {
        author_id: Some(owner),
        preview: title.clone(),
        title,
        link: Some(format!("/blog/{username}/{slug}")),
    }))
}

pub async fn comments_allowed(db: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT allow_comments FROM blog_posts WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .unwrap_or(false))
}

pub async fn owner_of(db: &PgPool, id: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar("SELECT b.owner_id FROM blog_posts p JOIN blogs b ON b.id = p.blog_id WHERE p.id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?)
}

pub async fn total_posts(db: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar(&format!("SELECT count(*) FROM blog_posts p WHERE {PUBLISHED}"))
        .fetch_one(db)
        .await?)
}

pub async fn user_post_count(db: &PgPool, user_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT count(*) FROM blog_posts p WHERE p.author_id = $1 AND {PUBLISHED}"
    ))
    .bind(user_id)
    .fetch_one(db)
    .await?)
}

// ---------------------------------------------------------------------------
// RSS
// ---------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    markdown::escape_html(s)
}

pub async fn rss(state: &AppState, username: Option<&str>) -> Result<String> {
    let site = state.settings();
    let (filter, title, link, description) = match username {
        Some(name) => {
            let blog = blog_by_username(&state.db, None, name).await?;
            (
                BlogFilter::Blog(blog.id),
                blog.title.clone(),
                state.config.url(&blog.url()),
                blog.description.clone(),
            )
        }
        None => (
            BlogFilter::All,
            format!("{} – Blogs", site.site_name),
            state.config.url("/blogs"),
            site.site_description.clone(),
        ),
    };
    let page = list_posts(&state.db, None, filter, None, Some(30)).await?;
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\"><channel>");
    out.push_str(&format!(
        "<title>{}</title><link>{}</link><description>{}</description><language>de</language>",
        xml_escape(&title),
        xml_escape(&link),
        xml_escape(&description)
    ));
    for p in page.items {
        let url = state.config.url(&p.url());
        let date = p.published_at.unwrap_or(p.created_at).to_rfc2822();
        out.push_str(&format!(
            "<item><title>{}</title><link>{}</link><guid isPermaLink=\"true\">{}</guid><pubDate>{}</pubDate>\
             <author>{}</author><description>{}</description></item>",
            xml_escape(&p.title),
            xml_escape(&url),
            xml_escape(&url),
            date,
            xml_escape(&p.author.display_name),
            xml_escape(&p.excerpt)
        ));
    }
    out.push_str("</channel></rss>");
    Ok(out)
}

pub async fn rss_all_handler(axum::extract::State(state): axum::extract::State<AppState>) -> axum::response::Response {
    rss_response(rss(&state, None).await)
}

pub async fn rss_blog_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(username): axum::extract::Path<String>,
) -> axum::response::Response {
    rss_response(rss(&state, Some(&username)).await)
}

fn rss_response(res: Result<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match res {
        Ok(xml) => ([(axum::http::header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")], xml).into_response(),
        Err(e) => e.into_response(),
    }
}
