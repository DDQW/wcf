//! Forum: categories, topics, threaded posts, read markers, subscriptions.

use std::collections::{HashMap, HashSet};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use shared::forum::{
    normalize_tags, BlockPage, CategoryAccess, CategoryInput, CategoryRef, CategoryView,
    ForumOverview, LastTopicInfo, NewTopic, PostRevision, PostView, ReplyPage, SubscriptionLevel,
    TagCount, ThreadBlock, TopicDetail, TopicFilter, TopicListItem, TopicPage, TopicSort,
    TopicUpdate, BLOCKS_PER_PAGE, CATEGORY_COLORS, REPLIES_PER_BLOCK, TOPICS_PER_PAGE,
};
use shared::user::ActivityItem;
use shared::ws::{ServerMsg, Topic};
use shared::{AuthUser, Cursor, NotificationKind, Role, SubjectType};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::users::{opt_user, user_cols, user_from};
use crate::{markdown, modlog, reactions};

fn flags(viewer: Option<&AuthUser>) -> (bool, bool) {
    (viewer.is_some(), viewer.is_some_and(|v| v.is_mod()))
}

fn role_of(viewer: Option<&AuthUser>) -> Option<Role> {
    viewer.map(|v| v.role)
}

/// Pushes the category visibility condition for alias `c`.
fn push_cat_visible(qb: &mut QueryBuilder<'_, Postgres>, viewer: Option<&AuthUser>) {
    let (logged_in, staff) = flags(viewer);
    qb.push(" (c.read_access = 'public' OR (c.read_access = 'members' AND ");
    qb.push_bind(logged_in);
    qb.push(") OR ");
    qb.push_bind(staff);
    qb.push(") ");
}

pub async fn can_read_topic(db: &PgPool, viewer: Option<&AuthUser>, topic_id: Uuid) -> Result<bool> {
    let row: Option<(String, bool)> = sqlx::query_as(
        "SELECT c.read_access, t.is_deleted FROM forum_topics t \
         JOIN forum_categories c ON c.id = t.category_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(db)
    .await?;
    let Some((access, deleted)) = row else { return Ok(false) };
    let (_, staff) = flags(viewer);
    Ok(CategoryAccess::parse(&access).allows(role_of(viewer)) && (!deleted || staff))
}

// ---------------------------------------------------------------------------
// Categories
// ---------------------------------------------------------------------------

const CATEGORY_SELECT: &str = "
    SELECT c.id, c.slug, c.name, c.description, c.color, c.position, c.parent_id,
           c.read_access, c.post_role,
           (SELECT count(*) FROM forum_topics t WHERE t.category_id = c.id AND NOT t.is_deleted) AS topic_count,
           (SELECT count(*) FROM forum_posts p JOIN forum_topics t ON t.id = p.topic_id
              WHERE t.category_id = c.id AND NOT t.is_deleted AND NOT p.is_deleted) AS post_count,
           lt.id AS lt_id, lt.title AS lt_title, lt.last_post_at AS lt_last_post_at,
           lu.id AS lu_id, lu.username::text AS lu_username, lu.display_name AS lu_display_name,
           lu.avatar_file_id AS lu_avatar, lu.role AS lu_role
    FROM forum_categories c
    LEFT JOIN LATERAL (
        SELECT t.id, t.title, t.last_post_at, t.last_poster_id FROM forum_topics t
        WHERE t.category_id = c.id AND NOT t.is_deleted
        ORDER BY t.last_post_at DESC LIMIT 1
    ) lt ON TRUE
    LEFT JOIN users lu ON lu.id = lt.last_poster_id
";

fn category_from(r: &PgRow) -> Result<CategoryView> {
    let read_access: String = r.try_get("read_access")?;
    let post_role: String = r.try_get("post_role")?;
    let lt_id: Option<Uuid> = r.try_get("lt_id")?;
    let last_topic = match lt_id {
        Some(id) => Some(LastTopicInfo {
            id,
            title: r.try_get("lt_title")?,
            last_post_at: r.try_get("lt_last_post_at")?,
            last_poster: opt_user(r, "lu")?,
        }),
        None => None,
    };
    Ok(CategoryView {
        id: r.try_get("id")?,
        slug: r.try_get("slug")?,
        name: r.try_get("name")?,
        description: r.try_get("description")?,
        color: r.try_get("color")?,
        position: r.try_get("position")?,
        parent_id: r.try_get("parent_id")?,
        read_access: CategoryAccess::parse(&read_access),
        post_role: Role::parse(&post_role).unwrap_or_default(),
        topic_count: r.try_get("topic_count")?,
        post_count: r.try_get("post_count")?,
        last_topic,
    })
}

pub async fn categories(db: &PgPool, viewer: Option<&AuthUser>) -> Result<Vec<CategoryView>> {
    let mut qb = QueryBuilder::new(CATEGORY_SELECT);
    qb.push(" WHERE ");
    push_cat_visible(&mut qb, viewer);
    qb.push(" ORDER BY c.position, c.name");
    let rows = qb.build().fetch_all(db).await?;
    rows.iter().map(category_from).collect()
}

pub async fn overview(db: &PgPool, viewer: Option<&AuthUser>) -> Result<ForumOverview> {
    let categories = categories(db, viewer).await?;
    let popular_tags = popular_tags(db, viewer, 30).await?;
    Ok(ForumOverview {
        categories,
        popular_tags,
    })
}

pub async fn category(db: &PgPool, viewer: Option<&AuthUser>, id: Uuid) -> Result<CategoryView> {
    let mut qb = QueryBuilder::new(CATEGORY_SELECT);
    qb.push(" WHERE c.id = ");
    qb.push_bind(id);
    qb.push(" AND ");
    push_cat_visible(&mut qb, viewer);
    let row = qb
        .build()
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Kategorie"))?;
    category_from(&row)
}

fn validate_category(input: &CategoryInput) -> Result<(String, Option<String>, String)> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(Error::invalid("Der Name muss 1–60 Zeichen lang sein."));
    }
    let description = input.description.trim();
    if description.chars().count() > 500 {
        return Err(Error::invalid("Die Beschreibung ist zu lang (max. 500 Zeichen)."));
    }
    let color = input.color.trim().to_lowercase();
    let valid_color = color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    let color = if valid_color { color } else { CATEGORY_COLORS[0].to_string() };
    Ok((
        name,
        (!description.is_empty()).then(|| description.to_string()),
        color,
    ))
}

async fn check_parent(db: &PgPool, id: Option<Uuid>, parent: Option<Uuid>) -> Result<()> {
    let Some(parent) = parent else { return Ok(()) };
    if Some(parent) == id {
        return Err(Error::invalid("Eine Kategorie kann nicht ihre eigene Oberkategorie sein."));
    }
    let parent_of_parent: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT parent_id FROM forum_categories WHERE id = $1")
            .bind(parent)
            .fetch_optional(db)
            .await?;
    match parent_of_parent {
        None => Err(Error::NotFound("Oberkategorie")),
        Some(Some(_)) => Err(Error::invalid("Unterkategorien können nicht weiter verschachtelt werden.")),
        Some(None) => Ok(()),
    }
}

pub async fn create_category(state: &AppState, actor: &AuthUser, input: CategoryInput) -> Result<Uuid> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    let (name, description, color) = validate_category(&input)?;
    check_parent(&state.db, None, input.parent_id).await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO forum_categories (id, slug, name, description, color, parent_id, read_access, post_role, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(markdown::slugify(&name, &id))
    .bind(&name)
    .bind(description)
    .bind(color)
    .bind(input.parent_id)
    .bind(input.read_access.as_str())
    .bind(input.post_role.as_str())
    .bind(input.position)
    .execute(&state.db)
    .await?;
    Ok(id)
}

pub async fn update_category(state: &AppState, actor: &AuthUser, id: Uuid, input: CategoryInput) -> Result<()> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    let (name, description, color) = validate_category(&input)?;
    check_parent(&state.db, Some(id), input.parent_id).await?;
    if input.parent_id.is_some() {
        let has_children: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM forum_categories WHERE parent_id = $1)")
                .bind(id)
                .fetch_one(&state.db)
                .await?;
        if has_children {
            return Err(Error::invalid(
                "Kategorien mit Unterkategorien können keine Unterkategorie werden.",
            ));
        }
    }
    let res = sqlx::query(
        "UPDATE forum_categories SET name = $2, description = $3, color = $4, parent_id = $5, \
            read_access = $6, post_role = $7, position = $8, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(&name)
    .bind(description)
    .bind(color)
    .bind(input.parent_id)
    .bind(input.read_access.as_str())
    .bind(input.post_role.as_str())
    .bind(input.position)
    .execute(&state.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(Error::NotFound("Kategorie"));
    }
    Ok(())
}

/// Deletes a category. Topics are moved to `move_to` first; without a
/// target the category must be empty.
pub async fn delete_category(state: &AppState, actor: &AuthUser, id: Uuid, move_to: Option<Uuid>) -> Result<()> {
    if !actor.is_admin() {
        return Err(Error::Forbidden);
    }
    let mut tx = state.db.begin().await?;
    if let Some(target) = move_to {
        if target == id {
            return Err(Error::invalid("Ziel und Quelle sind identisch."));
        }
        sqlx::query("UPDATE forum_topics SET category_id = $2 WHERE category_id = $1")
            .bind(id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
    } else {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM forum_topics WHERE category_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        if count > 0 {
            return Err(Error::invalid(
                "Die Kategorie enthält noch Themen – bitte ein Ziel zum Verschieben wählen.",
            ));
        }
    }
    sqlx::query("UPDATE forum_categories SET parent_id = NULL WHERE parent_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM forum_categories WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn popular_tags(db: &PgPool, viewer: Option<&AuthUser>, limit: i64) -> Result<Vec<TagCount>> {
    let mut qb = QueryBuilder::new(
        "SELECT tag, count(*) AS n FROM forum_topics t \
         JOIN forum_categories c ON c.id = t.category_id, unnest(t.tags) AS tag \
         WHERE NOT t.is_deleted AND ",
    );
    push_cat_visible(&mut qb, viewer);
    qb.push(" GROUP BY tag ORDER BY n DESC, tag LIMIT ");
    qb.push_bind(limit);
    let rows = qb.build().fetch_all(db).await?;
    rows.iter()
        .map(|r| {
            Ok(TagCount {
                tag: r.try_get("tag")?,
                count: r.try_get("n")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Topic lists
// ---------------------------------------------------------------------------

const LIKE_COUNT_SQL: &str = "(SELECT count(*) FROM reactions rx \
    WHERE rx.subject_type = 'forum_post' AND rx.subject_id = t.first_post_id)";

fn topic_select(viewer_id: Option<Uuid>) -> QueryBuilder<'static, Postgres> {
    let mut qb = QueryBuilder::new(format!(
        "SELECT t.id, t.title, t.slug, t.tags, t.is_pinned, t.is_locked, t.is_deleted, t.reply_count, \
                t.view_count, t.last_post_at, t.created_at, \
                c.id AS cat_id, c.name AS cat_name, c.color AS cat_color, {}, {}, \
                {LIKE_COUNT_SQL} AS like_count, \
                CASE WHEN r.user_id IS NULL THEN NULL ELSE ( \
                    SELECT count(*) FROM forum_posts p \
                    WHERE p.topic_id = t.id AND NOT p.is_deleted AND p.created_at > r.last_read_at \
                ) END AS unread_count \
         FROM forum_topics t \
         JOIN forum_categories c ON c.id = t.category_id \
         JOIN users a ON a.id = t.author_id \
         LEFT JOIN users lp ON lp.id = t.last_poster_id \
         LEFT JOIN forum_topic_reads r ON r.topic_id = t.id AND r.user_id = ",
        user_cols("a", "author"),
        user_cols("lp", "lp"),
    ));
    qb.push_bind(viewer_id);
    qb
}

fn topic_item_from(r: &PgRow) -> Result<TopicListItem> {
    Ok(TopicListItem {
        id: r.try_get("id")?,
        category: CategoryRef {
            id: r.try_get("cat_id")?,
            name: r.try_get("cat_name")?,
            color: r.try_get("cat_color")?,
        },
        title: r.try_get("title")?,
        slug: r.try_get("slug")?,
        author: user_from(r, "author")?,
        tags: r.try_get("tags")?,
        is_pinned: r.try_get("is_pinned")?,
        is_locked: r.try_get("is_locked")?,
        is_deleted: r.try_get("is_deleted")?,
        reply_count: r.try_get("reply_count")?,
        view_count: r.try_get("view_count")?,
        like_count: r.try_get("like_count")?,
        last_post_at: r.try_get("last_post_at")?,
        created_at: r.try_get("created_at")?,
        last_poster: opt_user(r, "lp")?,
        unread_count: r.try_get("unread_count")?,
    })
}

fn push_filter(qb: &mut QueryBuilder<'_, Postgres>, filter: &TopicFilter, viewer_id: Option<Uuid>) {
    match filter {
        TopicFilter::All => {}
        TopicFilter::Category(id) => {
            // A parent category also lists the topics of its subcategories.
            qb.push(" AND (t.category_id = ");
            qb.push_bind(*id);
            qb.push(" OR c.parent_id = ");
            qb.push_bind(*id);
            qb.push(")");
        }
        TopicFilter::Tag(tag) => {
            qb.push(" AND ");
            qb.push_bind(tag.clone());
            qb.push(" = ANY(t.tags)");
        }
        TopicFilter::Author(id) => {
            qb.push(" AND t.author_id = ");
            qb.push_bind(*id);
        }
        TopicFilter::Watching => {
            qb.push(
                " AND EXISTS (SELECT 1 FROM forum_topic_subs s WHERE s.topic_id = t.id \
                 AND s.level = 'watching' AND s.user_id = ",
            );
            qb.push_bind(viewer_id);
            qb.push(")");
        }
        TopicFilter::Unread => {
            qb.push(" AND r.user_id IS NOT NULL AND t.last_post_at > r.last_read_at");
        }
    }
}

pub async fn list_topics(
    db: &PgPool,
    viewer: Option<&AuthUser>,
    filter: TopicFilter,
    sort: TopicSort,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<TopicPage> {
    let viewer_id = viewer.map(|v| v.id);
    let limit = limit.unwrap_or(TOPICS_PER_PAGE).clamp(1, 100);
    if matches!(filter, TopicFilter::Watching | TopicFilter::Unread) && viewer.is_none() {
        return Ok(TopicPage { items: Vec::new(), next_cursor: None });
    }
    let pinned_first = matches!(filter, TopicFilter::Category(_)) && sort == TopicSort::Activity;

    let mut items = Vec::new();
    if pinned_first && cursor.is_none() {
        let mut qb = topic_select(viewer_id);
        qb.push(" WHERE NOT t.is_deleted AND t.is_pinned AND ");
        push_cat_visible(&mut qb, viewer);
        push_filter(&mut qb, &filter, viewer_id);
        qb.push(" ORDER BY t.last_post_at DESC");
        for r in qb.build().fetch_all(db).await? {
            items.push(topic_item_from(&r)?);
        }
    }

    let mut qb = topic_select(viewer_id);
    qb.push(" WHERE NOT t.is_deleted AND ");
    push_cat_visible(&mut qb, viewer);
    push_filter(&mut qb, &filter, viewer_id);
    if pinned_first {
        qb.push(" AND NOT t.is_pinned");
    }
    let mut offset = 0i64;
    match sort {
        TopicSort::Activity | TopicSort::Newest => {
            let col = if sort == TopicSort::Activity { "t.last_post_at" } else { "t.created_at" };
            if let Some(c) = cursor.as_deref().and_then(|c| Cursor::decode(c).ok()) {
                qb.push(format!(" AND ({col}, t.id) < ("));
                qb.push_bind(c.ts);
                qb.push(", ");
                qb.push_bind(c.id);
                qb.push(")");
            }
            qb.push(format!(" ORDER BY {col} DESC, t.id DESC"));
        }
        TopicSort::Top => {
            offset = cursor
                .as_deref()
                .and_then(|c| c.strip_prefix('o'))
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            qb.push(format!(
                " ORDER BY ({LIKE_COUNT_SQL} * 2 + t.reply_count + t.view_count / 20) DESC, t.created_at DESC"
            ));
        }
    }
    qb.push(" LIMIT ");
    qb.push_bind(limit + 1);
    if offset > 0 {
        qb.push(" OFFSET ");
        qb.push_bind(offset);
    }
    let rows = qb.build().fetch_all(db).await?;
    let mut page: Vec<TopicListItem> = rows.iter().map(topic_item_from).collect::<Result<_>>()?;
    let more = page.len() as i64 > limit;
    page.truncate(limit as usize);
    let next_cursor = if more {
        match sort {
            TopicSort::Activity => page.last().map(|t| Cursor::new(t.last_post_at, t.id).encode()),
            TopicSort::Newest => page.last().map(|t| Cursor::new(t.created_at, t.id).encode()),
            TopicSort::Top => Some(format!("o{}", offset + limit)),
        }
    } else {
        None
    };
    items.extend(page);
    Ok(TopicPage { items, next_cursor })
}

// ---------------------------------------------------------------------------
// Topic detail
// ---------------------------------------------------------------------------

struct TopicMeta {
    author_id: Uuid,
    title: String,
    is_locked: bool,
    is_deleted: bool,
    read_access: CategoryAccess,
    first_post_id: Option<Uuid>,
}

async fn topic_meta(db: &PgPool, topic_id: Uuid) -> Result<TopicMeta> {
    let r = sqlx::query(
        "SELECT t.category_id, t.author_id, t.title, t.is_locked, t.is_deleted, t.first_post_id, c.read_access \
         FROM forum_topics t JOIN forum_categories c ON c.id = t.category_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Thema"))?;
    let access: String = r.try_get("read_access")?;
    Ok(TopicMeta {
        author_id: r.try_get("author_id")?,
        title: r.try_get("title")?,
        is_locked: r.try_get("is_locked")?,
        is_deleted: r.try_get("is_deleted")?,
        read_access: CategoryAccess::parse(&access),
        first_post_id: r.try_get("first_post_id")?,
    })
}

fn ensure_readable(meta: &TopicMeta, viewer: Option<&AuthUser>) -> Result<()> {
    let (_, staff) = flags(viewer);
    if !meta.read_access.allows(role_of(viewer)) || (meta.is_deleted && !staff) {
        return Err(Error::NotFound("Thema"));
    }
    Ok(())
}

/// Loads a topic for display. For logged-in viewers this also advances the
/// read marker (returning the previous one) and counts the view.
pub async fn get_topic(state: &AppState, viewer: Option<&AuthUser>, topic_id: Uuid) -> Result<TopicDetail> {
    let db = &state.db;
    let viewer_id = viewer.map(|v| v.id);
    let r = sqlx::query(&format!(
        "SELECT t.id, t.title, t.slug, t.tags, t.is_pinned, t.is_locked, t.is_deleted, t.reply_count, \
                t.view_count, t.created_at, t.last_post_at, t.first_post_id, \
                c.id AS cat_id, c.name AS cat_name, c.color AS cat_color, c.read_access, {}, \
                r.last_read_at, s.level AS sub_level \
         FROM forum_topics t \
         JOIN forum_categories c ON c.id = t.category_id \
         JOIN users a ON a.id = t.author_id \
         LEFT JOIN forum_topic_reads r ON r.topic_id = t.id AND r.user_id = $2 \
         LEFT JOIN forum_topic_subs s ON s.topic_id = t.id AND s.user_id = $2 \
         WHERE t.id = $1",
        user_cols("a", "author")
    ))
    .bind(topic_id)
    .bind(viewer_id)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Thema"))?;

    let access: String = r.try_get("read_access")?;
    let is_deleted: bool = r.try_get("is_deleted")?;
    let is_locked: bool = r.try_get("is_locked")?;
    let (logged_in, staff) = flags(viewer);
    if !CategoryAccess::parse(&access).allows(role_of(viewer)) || (is_deleted && !staff) {
        return Err(Error::NotFound("Thema"));
    }
    let author = user_from(&r, "author")?;
    let last_read_at: Option<DateTime<Utc>> = r.try_get("last_read_at")?;
    let sub_level: Option<String> = r.try_get("sub_level")?;

    let mut view_count: i32 = r.try_get("view_count")?;
    let count_view = match (viewer_id, last_read_at) {
        (None, _) | (Some(_), None) => true,
        (Some(_), Some(t)) => Utc::now() - t > chrono::Duration::minutes(30),
    };
    if count_view {
        sqlx::query("UPDATE forum_topics SET view_count = view_count + 1 WHERE id = $1")
            .bind(topic_id)
            .execute(db)
            .await?;
        view_count += 1;
    }
    let mut first_unread_post = None;
    if let Some(uid) = viewer_id {
        if let Some(t) = last_read_at {
            first_unread_post = sqlx::query_scalar(
                "SELECT id FROM forum_posts WHERE topic_id = $1 AND created_at > $2 AND NOT is_deleted \
                 ORDER BY created_at, id LIMIT 1",
            )
            .bind(topic_id)
            .bind(t)
            .fetch_optional(db)
            .await?;
        }
        sqlx::query(
            "INSERT INTO forum_topic_reads (user_id, topic_id, last_read_at) VALUES ($1, $2, now()) \
             ON CONFLICT (user_id, topic_id) DO UPDATE SET last_read_at = now()",
        )
        .bind(uid)
        .bind(topic_id)
        .execute(db)
        .await?;
        notifications::mark_link_read(state, uid, &format!("/forum/t/{topic_id}")).await?;
    }

    Ok(TopicDetail {
        id: r.try_get("id")?,
        category: CategoryRef {
            id: r.try_get("cat_id")?,
            name: r.try_get("cat_name")?,
            color: r.try_get("cat_color")?,
        },
        title: r.try_get("title")?,
        slug: r.try_get("slug")?,
        can_edit: viewer.is_some_and(|v| v.id == author.id || v.is_mod()),
        author,
        tags: r.try_get("tags")?,
        is_pinned: r.try_get("is_pinned")?,
        is_locked,
        is_deleted,
        reply_count: r.try_get("reply_count")?,
        view_count,
        created_at: r.try_get("created_at")?,
        last_post_at: r.try_get("last_post_at")?,
        subscription: sub_level
            .map(|l| SubscriptionLevel::parse(&l))
            .unwrap_or_default(),
        last_read_at,
        first_unread_post,
        first_post_id: r.try_get("first_post_id")?,
        can_reply: logged_in && !is_deleted && (!is_locked || staff),
        can_moderate: staff,
    })
}

// ---------------------------------------------------------------------------
// Posts & thread blocks
// ---------------------------------------------------------------------------

fn post_cols() -> String {
    format!(
        "p.id, p.topic_id, p.parent_post_id, p.thread_root_id, p.depth, p.body_html, p.created_at, \
         p.edited_at, p.edit_count, p.is_deleted, {}, pu.display_name AS reply_to",
        user_cols("u", "author")
    )
}

const POST_JOINS: &str = " FROM forum_posts p JOIN users u ON u.id = p.author_id \
    LEFT JOIN forum_posts pp ON pp.id = p.parent_post_id \
    LEFT JOIN users pu ON pu.id = pp.author_id ";

fn post_from(r: &PgRow, staff: bool) -> Result<PostView> {
    let is_deleted: bool = r.try_get("is_deleted")?;
    let body_html: String = r.try_get("body_html")?;
    Ok(PostView {
        id: r.try_get("id")?,
        topic_id: r.try_get("topic_id")?,
        author: user_from(r, "author")?,
        parent_post_id: r.try_get("parent_post_id")?,
        thread_root_id: r.try_get("thread_root_id")?,
        depth: r.try_get("depth")?,
        body_html: if is_deleted && !staff { String::new() } else { body_html },
        created_at: r.try_get("created_at")?,
        edited_at: r.try_get("edited_at")?,
        edit_count: r.try_get("edit_count")?,
        is_deleted,
        reactions: Vec::new(),
        reply_to: r.try_get("reply_to")?,
    })
}

async fn attach_reactions(db: &PgPool, posts: &mut [PostView], viewer: Option<Uuid>) -> Result<()> {
    let ids: Vec<Uuid> = posts.iter().map(|p| p.id).collect();
    let mut counts = reactions::counts_for(db, SubjectType::ForumPost, &ids, viewer).await?;
    for p in posts.iter_mut() {
        p.reactions = counts.remove(&p.id).unwrap_or_default();
    }
    Ok(())
}

pub async fn post_view(db: &PgPool, viewer: Option<&AuthUser>, post_id: Uuid) -> Result<PostView> {
    let (_, staff) = flags(viewer);
    let r = sqlx::query(&format!("SELECT {}{POST_JOINS} WHERE p.id = $1", post_cols()))
        .bind(post_id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Beitrag"))?;
    let mut posts = vec![post_from(&r, staff)?];
    attach_reactions(db, &mut posts, viewer.map(|v| v.id)).await?;
    Ok(posts.remove(0))
}

/// Which page of thread blocks to load.
#[derive(Debug, Clone, Default)]
pub struct BlockQuery {
    /// Continue after this cursor (older → newer).
    pub after: Option<String>,
    /// Load the page before this cursor.
    pub before: Option<String>,
    /// Start with the block that contains this post (permalinks, first unread).
    pub from_post: Option<Uuid>,
}

const ROOT_VISIBLE: &str = "(NOT p.is_deleted OR $2 OR EXISTS (SELECT 1 FROM forum_posts ch \
    WHERE ch.thread_root_id = p.id AND ch.id <> p.id AND NOT ch.is_deleted))";

pub async fn list_blocks(
    db: &PgPool,
    viewer: Option<&AuthUser>,
    topic_id: Uuid,
    q: BlockQuery,
) -> Result<BlockPage> {
    let meta = topic_meta(db, topic_id).await?;
    ensure_readable(&meta, viewer)?;
    let (_, staff) = flags(viewer);
    let n = BLOCKS_PER_PAGE;
    let base = format!(
        "SELECT {}{POST_JOINS} WHERE p.topic_id = $1 AND p.parent_post_id IS NULL AND {ROOT_VISIBLE}",
        post_cols()
    );

    let mut next_cursor = None;
    let mut prev_cursor = None;
    let roots: Vec<PostView>;

    if let Some(c) = q.before.as_deref().and_then(|c| Cursor::decode(c).ok()) {
        let rows = sqlx::query(&format!(
            "{base} AND (p.created_at, p.id) < ($3, $4) ORDER BY p.created_at DESC, p.id DESC LIMIT $5"
        ))
        .bind(topic_id)
        .bind(staff)
        .bind(c.ts)
        .bind(c.id)
        .bind(n + 1)
        .fetch_all(db)
        .await?;
        let mut list: Vec<PostView> = rows.iter().map(|r| post_from(r, staff)).collect::<Result<_>>()?;
        if list.len() as i64 > n {
            list.truncate(n as usize);
            prev_cursor = list.last().map(|p| Cursor::new(p.created_at, p.id).encode());
        }
        list.reverse();
        roots = list;
    } else {
        let start: Option<(DateTime<Utc>, Uuid, bool)> = if let Some(post_id) = q.from_post {
            let root: Option<(DateTime<Utc>, Uuid)> = sqlx::query_as(
                "SELECT r.created_at, r.id FROM forum_posts p JOIN forum_posts r ON r.id = p.thread_root_id \
                 WHERE p.id = $1 AND p.topic_id = $2",
            )
            .bind(post_id)
            .bind(topic_id)
            .fetch_optional(db)
            .await?;
            root.map(|(ts, id)| (ts, id, true))
        } else {
            q.after
                .as_deref()
                .and_then(|c| Cursor::decode(c).ok())
                .map(|c| (c.ts, c.id, false))
        };
        let rows = match start {
            Some((ts, id, inclusive)) => {
                let op = if inclusive { ">=" } else { ">" };
                sqlx::query(&format!(
                    "{base} AND (p.created_at, p.id) {op} ($3, $4) ORDER BY p.created_at, p.id LIMIT $5"
                ))
                .bind(topic_id)
                .bind(staff)
                .bind(ts)
                .bind(id)
                .bind(n + 1)
                .fetch_all(db)
                .await?
            }
            None => {
                sqlx::query(&format!("{base} ORDER BY p.created_at, p.id LIMIT $3"))
                    .bind(topic_id)
                    .bind(staff)
                    .bind(n + 1)
                    .fetch_all(db)
                    .await?
            }
        };
        let mut list: Vec<PostView> = rows.iter().map(|r| post_from(r, staff)).collect::<Result<_>>()?;
        if list.len() as i64 > n {
            list.truncate(n as usize);
            next_cursor = list.last().map(|p| Cursor::new(p.created_at, p.id).encode());
        }
        if q.from_post.is_some() {
            if let Some(first) = list.first() {
                let earlier: bool = sqlx::query_scalar(&format!(
                    "SELECT EXISTS ({base} AND (p.created_at, p.id) < ($3, $4))"
                ))
                .bind(topic_id)
                .bind(staff)
                .bind(first.created_at)
                .bind(first.id)
                .fetch_one(db)
                .await?;
                if earlier {
                    prev_cursor = Some(Cursor::new(first.created_at, first.id).encode());
                }
            }
        }
        roots = list;
    }

    // Replies for all roots of this page, capped per thread.
    let root_ids: Vec<Uuid> = roots.iter().map(|p| p.id).collect();
    let reply_rows = sqlx::query(&format!(
        "SELECT * FROM ( \
            SELECT {}, row_number() OVER w AS rn, count(*) OVER (PARTITION BY p.thread_root_id) AS total \
            {POST_JOINS} \
            WHERE p.thread_root_id = ANY($1) AND p.id <> p.thread_root_id \
              AND (NOT p.is_deleted OR $2 OR EXISTS (SELECT 1 FROM forum_posts ch \
                   WHERE ch.parent_post_id = p.id AND NOT ch.is_deleted)) \
            WINDOW w AS (PARTITION BY p.thread_root_id ORDER BY p.created_at, p.id) \
         ) x WHERE rn <= $3 ORDER BY created_at, id",
        post_cols()
    ))
    .bind(&root_ids)
    .bind(staff)
    .bind(REPLIES_PER_BLOCK)
    .fetch_all(db)
    .await?;
    let mut replies_by_root: HashMap<Uuid, (Vec<PostView>, i64)> = HashMap::new();
    for r in &reply_rows {
        let p = post_from(r, staff)?;
        let total: i64 = r.try_get("total")?;
        let entry = replies_by_root.entry(p.thread_root_id).or_insert((Vec::new(), total));
        entry.0.push(p);
    }

    let viewer_id = viewer.map(|v| v.id);
    let mut all: Vec<PostView> = roots;
    let root_count = all.len();
    for id in &root_ids {
        if let Some((list, _)) = replies_by_root.get(id) {
            all.extend(list.iter().cloned());
        }
    }
    attach_reactions(db, &mut all, viewer_id).await?;
    let mut by_id: HashMap<Uuid, PostView> = all.into_iter().map(|p| (p.id, p)).collect();

    let mut blocks = Vec::with_capacity(root_count);
    for id in root_ids {
        let Some(root) = by_id.remove(&id) else { continue };
        let (reply_list, total) = replies_by_root.remove(&id).unwrap_or_default();
        let replies = reply_list
            .into_iter()
            .filter_map(|p| by_id.remove(&p.id))
            .collect();
        blocks.push(ThreadBlock {
            root,
            replies,
            reply_total: total,
        });
    }
    Ok(BlockPage {
        blocks,
        next_cursor,
        prev_cursor,
    })
}

/// More replies of one thread block, after a cursor.
pub async fn list_replies(
    db: &PgPool,
    viewer: Option<&AuthUser>,
    root_id: Uuid,
    after: Option<String>,
) -> Result<ReplyPage> {
    let topic_id: Uuid = sqlx::query_scalar("SELECT topic_id FROM forum_posts WHERE id = $1")
        .bind(root_id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Beitrag"))?;
    let meta = topic_meta(db, topic_id).await?;
    ensure_readable(&meta, viewer)?;
    let (_, staff) = flags(viewer);
    let limit = 50i64;
    let cursor = after.as_deref().and_then(|c| Cursor::decode(c).ok());
    let rows = sqlx::query(&format!(
        "SELECT {}{POST_JOINS} WHERE p.thread_root_id = $1 AND p.id <> p.thread_root_id \
           AND (NOT p.is_deleted OR $2 OR EXISTS (SELECT 1 FROM forum_posts ch \
                WHERE ch.parent_post_id = p.id AND NOT ch.is_deleted)) \
           AND ($3::timestamptz IS NULL OR (p.created_at, p.id) > ($3, $4)) \
         ORDER BY p.created_at, p.id LIMIT $5",
        post_cols()
    ))
    .bind(root_id)
    .bind(staff)
    .bind(cursor.map(|c| c.ts))
    .bind(cursor.map(|c| c.id))
    .bind(limit + 1)
    .fetch_all(db)
    .await?;
    let mut replies: Vec<PostView> = rows.iter().map(|r| post_from(r, staff)).collect::<Result<_>>()?;
    let next_cursor = if replies.len() as i64 > limit {
        replies.truncate(limit as usize);
        replies.last().map(|p| Cursor::new(p.created_at, p.id).encode())
    } else {
        None
    };
    attach_reactions(db, &mut replies, viewer.map(|v| v.id)).await?;
    Ok(ReplyPage { replies, next_cursor })
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn validate_title(title: &str) -> Result<String> {
    let title = title.trim();
    let len = title.chars().count();
    if !(3..=200).contains(&len) {
        return Err(Error::invalid("Der Titel muss 3–200 Zeichen lang sein."));
    }
    Ok(title.to_string())
}

fn validate_body(body: &str) -> Result<()> {
    if body.trim().is_empty() {
        return Err(Error::invalid("Der Text darf nicht leer sein."));
    }
    if body.chars().count() > 50_000 {
        return Err(Error::invalid("Der Text ist zu lang (max. 50.000 Zeichen)."));
    }
    Ok(())
}

pub async fn create_topic(state: &AppState, user: &AuthUser, input: NewTopic) -> Result<Uuid> {
    state
        .limiter
        .hit(&format!("topic:{}", user.id), 5, StdDuration::from_secs(600))?;
    let title = validate_title(&input.title)?;
    validate_body(&input.body_md)?;
    let tags = normalize_tags(&input.tags);

    let cat: Option<(String, String)> =
        sqlx::query_as("SELECT read_access, post_role FROM forum_categories WHERE id = $1")
            .bind(input.category_id)
            .fetch_optional(&state.db)
            .await?;
    let (access, post_role) = cat.ok_or(Error::NotFound("Kategorie"))?;
    if !CategoryAccess::parse(&access).allows(Some(user.role)) {
        return Err(Error::NotFound("Kategorie"));
    }
    if !user.role.at_least(Role::parse(&post_role).unwrap_or_default()) {
        return Err(Error::invalid("In dieser Kategorie dürfen nur Moderatoren Themen erstellen."));
    }

    let rendered = markdown::render_mentions(&state.db, &input.body_md).await?;
    let topic_id = Uuid::now_v7();
    let post_id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO forum_topics (id, category_id, author_id, title, slug, tags, last_poster_id, first_post_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $3, $7)",
    )
    .bind(topic_id)
    .bind(input.category_id)
    .bind(user.id)
    .bind(&title)
    .bind(markdown::slugify(&title, &topic_id))
    .bind(&tags)
    .bind(post_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO forum_posts (id, topic_id, author_id, parent_post_id, thread_root_id, depth, body_md, body_html) \
         VALUES ($1, $2, $3, NULL, $1, 0, $4, $5)",
    )
    .bind(post_id)
    .bind(topic_id)
    .bind(user.id)
    .bind(&input.body_md)
    .bind(&rendered.html)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO forum_topic_subs (user_id, topic_id, level) VALUES ($1, $2, 'watching')",
    )
    .bind(user.id)
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO forum_topic_reads (user_id, topic_id, last_read_at) VALUES ($1, $2, now())")
        .bind(user.id)
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let excerpt = markdown::excerpt(&input.body_md, 160);
    for uid in rendered.mentioned {
        notifications::notify(
            state,
            NewNotification {
                user_id: uid,
                kind: NotificationKind::Mention,
                actor_id: Some(user.id),
                title: format!("{} hat dich in „{}“ erwähnt", user.display_name, title),
                body: excerpt.clone(),
                link: format!("/forum/t/{topic_id}?post={post_id}"),
                subject: Some((SubjectType::ForumPost, post_id)),
            },
        )
        .await;
    }
    Ok(topic_id)
}

async fn sub_level(db: &PgPool, user_id: Uuid, topic_id: Uuid) -> Result<SubscriptionLevel> {
    let level: Option<String> =
        sqlx::query_scalar("SELECT level FROM forum_topic_subs WHERE user_id = $1 AND topic_id = $2")
            .bind(user_id)
            .bind(topic_id)
            .fetch_optional(db)
            .await?;
    Ok(level.map(|l| SubscriptionLevel::parse(&l)).unwrap_or_default())
}

pub async fn create_post(
    state: &AppState,
    user: &AuthUser,
    topic_id: Uuid,
    parent_id: Option<Uuid>,
    body_md: &str,
) -> Result<PostView> {
    state
        .limiter
        .hit(&format!("post:{}", user.id), 10, StdDuration::from_secs(60))?;
    validate_body(body_md)?;
    let meta = topic_meta(&state.db, topic_id).await?;
    ensure_readable(&meta, Some(user))?;
    if meta.is_deleted {
        return Err(Error::invalid("Das Thema wurde gelöscht."));
    }
    if meta.is_locked && !user.is_mod() {
        return Err(Error::invalid("Das Thema ist gesperrt."));
    }

    let post_id = Uuid::now_v7();
    let (thread_root_id, depth, parent_author) = match parent_id {
        Some(pid) => {
            let parent: Option<(Uuid, i16, Uuid, bool)> = sqlx::query_as(
                "SELECT thread_root_id, depth, author_id, is_deleted FROM forum_posts \
                 WHERE id = $1 AND topic_id = $2",
            )
            .bind(pid)
            .bind(topic_id)
            .fetch_optional(&state.db)
            .await?;
            let (root, d, author, deleted) = parent.ok_or(Error::NotFound("Beitrag"))?;
            if deleted {
                return Err(Error::invalid("Auf gelöschte Beiträge kann nicht geantwortet werden."));
            }
            (root, d.saturating_add(1), Some(author))
        }
        None => (post_id, 0, None),
    };

    let rendered = markdown::render_mentions(&state.db, body_md).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO forum_posts (id, topic_id, author_id, parent_post_id, thread_root_id, depth, body_md, body_html) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(post_id)
    .bind(topic_id)
    .bind(user.id)
    .bind(parent_id)
    .bind(thread_root_id)
    .bind(depth)
    .bind(body_md)
    .bind(&rendered.html)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE forum_topics SET reply_count = reply_count + 1, last_post_at = now(), \
            last_poster_id = $2, updated_at = now() WHERE id = $1",
    )
    .bind(topic_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO forum_topic_reads (user_id, topic_id, last_read_at) VALUES ($1, $2, now()) \
         ON CONFLICT (user_id, topic_id) DO UPDATE SET last_read_at = now()",
    )
    .bind(user.id)
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Notifications: direct reply target, then mentions, then watchers.
    let link = format!("/forum/t/{topic_id}?post={post_id}");
    let excerpt = markdown::excerpt(body_md, 160);
    let mut notified: HashSet<Uuid> = HashSet::from([user.id]);
    let target = parent_author.unwrap_or(meta.author_id);
    if !notified.contains(&target) && sub_level(&state.db, target, topic_id).await? != SubscriptionLevel::Muted {
        notified.insert(target);
        notifications::notify(
            state,
            NewNotification {
                user_id: target,
                kind: NotificationKind::Reply,
                actor_id: Some(user.id),
                title: format!("{} hat in „{}“ geantwortet", user.display_name, meta.title),
                body: excerpt.clone(),
                link: link.clone(),
                subject: Some((SubjectType::ForumPost, post_id)),
            },
        )
        .await;
    }
    for uid in rendered.mentioned {
        if notified.insert(uid) {
            notifications::notify(
                state,
                NewNotification {
                    user_id: uid,
                    kind: NotificationKind::Mention,
                    actor_id: Some(user.id),
                    title: format!("{} hat dich in „{}“ erwähnt", user.display_name, meta.title),
                    body: excerpt.clone(),
                    link: link.clone(),
                    subject: Some((SubjectType::ForumPost, post_id)),
                },
            )
            .await;
        }
    }
    let watchers: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM forum_topic_subs WHERE topic_id = $1 AND level = 'watching'",
    )
    .bind(topic_id)
    .fetch_all(&state.db)
    .await?;
    for uid in watchers {
        if notified.insert(uid) {
            notifications::notify(
                state,
                NewNotification {
                    user_id: uid,
                    kind: NotificationKind::Subscription,
                    actor_id: Some(user.id),
                    title: format!("Neuer Beitrag in „{}“", meta.title),
                    body: format!("{}: {}", user.display_name, excerpt),
                    link: link.clone(),
                    subject: Some((SubjectType::ForumPost, post_id)),
                },
            )
            .await;
        }
    }

    let post = post_view(&state.db, Some(user), post_id).await?;
    state
        .hub
        .publish(&Topic::ForumTopic(topic_id), &ServerMsg::ForumPostCreated { post: post.clone() });
    Ok(post)
}

struct PostMeta {
    topic_id: Uuid,
    author_id: Uuid,
    body_md: String,
    body_html: String,
    is_deleted: bool,
}

async fn post_meta(db: &PgPool, post_id: Uuid) -> Result<PostMeta> {
    let r = sqlx::query("SELECT topic_id, author_id, body_md, body_html, is_deleted FROM forum_posts WHERE id = $1")
        .bind(post_id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Beitrag"))?;
    Ok(PostMeta {
        topic_id: r.try_get("topic_id")?,
        author_id: r.try_get("author_id")?,
        body_md: r.try_get("body_md")?,
        body_html: r.try_get("body_html")?,
        is_deleted: r.try_get("is_deleted")?,
    })
}

/// Markdown source of a post (for quoting and editing).
pub async fn post_source(db: &PgPool, viewer: Option<&AuthUser>, post_id: Uuid) -> Result<String> {
    let pm = post_meta(db, post_id).await?;
    let meta = topic_meta(db, pm.topic_id).await?;
    ensure_readable(&meta, viewer)?;
    if pm.is_deleted && !flags(viewer).1 {
        return Err(Error::NotFound("Beitrag"));
    }
    Ok(pm.body_md)
}

pub async fn edit_post(state: &AppState, user: &AuthUser, post_id: Uuid, body_md: &str) -> Result<PostView> {
    validate_body(body_md)?;
    let pm = post_meta(&state.db, post_id).await?;
    let meta = topic_meta(&state.db, pm.topic_id).await?;
    ensure_readable(&meta, Some(user))?;
    let is_author = pm.author_id == user.id;
    if !(is_author || user.is_mod()) {
        return Err(Error::Forbidden);
    }
    if pm.is_deleted {
        return Err(Error::invalid("Gelöschte Beiträge können nicht bearbeitet werden."));
    }
    if meta.is_locked && !user.is_mod() {
        return Err(Error::invalid("Das Thema ist gesperrt."));
    }
    if pm.body_md == body_md {
        return post_view(&state.db, Some(user), post_id).await;
    }
    let old_mentions: HashSet<Uuid> = markdown::render_mentions(&state.db, &pm.body_md)
        .await?
        .mentioned
        .into_iter()
        .collect();
    let rendered = markdown::render_mentions(&state.db, body_md).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO forum_post_revisions (id, post_id, body_md, body_html, editor_id) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(post_id)
    .bind(&pm.body_md)
    .bind(&pm.body_html)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE forum_posts SET body_md = $2, body_html = $3, edited_at = now(), \
            edit_count = edit_count + 1, updated_at = now() WHERE id = $1",
    )
    .bind(post_id)
    .bind(body_md)
    .bind(&rendered.html)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    for uid in rendered.mentioned.into_iter().filter(|u| !old_mentions.contains(u)) {
        notifications::notify(
            state,
            NewNotification {
                user_id: uid,
                kind: NotificationKind::Mention,
                actor_id: Some(user.id),
                title: format!("{} hat dich in „{}“ erwähnt", user.display_name, meta.title),
                body: markdown::excerpt(body_md, 160),
                link: format!("/forum/t/{}?post={post_id}", pm.topic_id),
                subject: Some((SubjectType::ForumPost, post_id)),
            },
        )
        .await;
    }
    if !is_author {
        modlog::record(&state.db, Some(user.id), "edit_content", Some((SubjectType::ForumPost, post_id)), Some(pm.author_id), "").await?;
    }
    let post = post_view(&state.db, Some(user), post_id).await?;
    state
        .hub
        .publish(&Topic::ForumTopic(pm.topic_id), &ServerMsg::ForumPostUpdated { post: post.clone() });
    Ok(post)
}

async fn recount_replies(db: &PgPool, topic_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE forum_topics SET reply_count = GREATEST(0, \
            (SELECT count(*) FROM forum_posts WHERE topic_id = $1 AND NOT is_deleted) - 1) \
         WHERE id = $1",
    )
    .bind(topic_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_post(state: &AppState, user: &AuthUser, post_id: Uuid) -> Result<()> {
    let pm = post_meta(&state.db, post_id).await?;
    let meta = topic_meta(&state.db, pm.topic_id).await?;
    ensure_readable(&meta, Some(user))?;
    let is_author = pm.author_id == user.id;
    if !(is_author || user.is_mod()) {
        return Err(Error::Forbidden);
    }
    if meta.first_post_id == Some(post_id) {
        return delete_topic(state, user, pm.topic_id).await;
    }
    if pm.is_deleted {
        return Ok(());
    }
    sqlx::query("UPDATE forum_posts SET is_deleted = TRUE, deleted_by = $2, updated_at = now() WHERE id = $1")
        .bind(post_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    recount_replies(&state.db, pm.topic_id).await?;
    if !is_author {
        modlog::record(&state.db, Some(user.id), "delete_content", Some((SubjectType::ForumPost, post_id)), Some(pm.author_id), &markdown::excerpt(&pm.body_md, 120)).await?;
    }
    state.hub.publish(
        &Topic::ForumTopic(pm.topic_id),
        &ServerMsg::ForumPostDeleted {
            topic_id: pm.topic_id,
            post_id,
        },
    );
    Ok(())
}

pub async fn restore_post(state: &AppState, user: &AuthUser, post_id: Uuid) -> Result<PostView> {
    if !user.is_mod() {
        return Err(Error::Forbidden);
    }
    let pm = post_meta(&state.db, post_id).await?;
    sqlx::query("UPDATE forum_posts SET is_deleted = FALSE, deleted_by = NULL WHERE id = $1")
        .bind(post_id)
        .execute(&state.db)
        .await?;
    recount_replies(&state.db, pm.topic_id).await?;
    modlog::record(&state.db, Some(user.id), "restore_content", Some((SubjectType::ForumPost, post_id)), Some(pm.author_id), "").await?;
    let post = post_view(&state.db, Some(user), post_id).await?;
    state
        .hub
        .publish(&Topic::ForumTopic(pm.topic_id), &ServerMsg::ForumPostUpdated { post: post.clone() });
    Ok(post)
}

pub async fn post_revisions(db: &PgPool, viewer: Option<&AuthUser>, post_id: Uuid) -> Result<Vec<PostRevision>> {
    let pm = post_meta(db, post_id).await?;
    let meta = topic_meta(db, pm.topic_id).await?;
    ensure_readable(&meta, viewer)?;
    let rows = sqlx::query(&format!(
        "SELECT r.id, r.body_html, r.created_at, {} FROM forum_post_revisions r \
         JOIN users u ON u.id = r.editor_id WHERE r.post_id = $1 ORDER BY r.created_at DESC",
        user_cols("u", "editor")
    ))
    .bind(post_id)
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(PostRevision {
                id: r.try_get("id")?,
                body_html: r.try_get("body_html")?,
                editor: user_from(r, "editor")?,
                created_at: r.try_get("created_at")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Topic management
// ---------------------------------------------------------------------------

pub async fn update_topic(state: &AppState, user: &AuthUser, topic_id: Uuid, input: TopicUpdate) -> Result<()> {
    let meta = topic_meta(&state.db, topic_id).await?;
    ensure_readable(&meta, Some(user))?;
    if !(meta.author_id == user.id || user.is_mod()) {
        return Err(Error::Forbidden);
    }
    if meta.is_locked && !user.is_mod() {
        return Err(Error::invalid("Das Thema ist gesperrt."));
    }
    let title = validate_title(&input.title)?;
    let tags = normalize_tags(&input.tags);
    sqlx::query("UPDATE forum_topics SET title = $2, tags = $3, updated_at = now() WHERE id = $1")
        .bind(topic_id)
        .bind(&title)
        .bind(&tags)
        .execute(&state.db)
        .await?;
    state
        .hub
        .publish(&Topic::ForumTopic(topic_id), &ServerMsg::ForumTopicUpdated { topic_id });
    Ok(())
}

async fn set_flag(state: &AppState, user: &AuthUser, topic_id: Uuid, column: &str, value: bool, action: &str) -> Result<()> {
    if !user.is_mod() {
        return Err(Error::Forbidden);
    }
    let meta = topic_meta(&state.db, topic_id).await?;
    sqlx::query(&format!("UPDATE forum_topics SET {column} = $2 WHERE id = $1"))
        .bind(topic_id)
        .bind(value)
        .execute(&state.db)
        .await?;
    modlog::record(&state.db, Some(user.id), action, Some((SubjectType::ForumTopic, topic_id)), Some(meta.author_id), &meta.title).await?;
    state
        .hub
        .publish(&Topic::ForumTopic(topic_id), &ServerMsg::ForumTopicUpdated { topic_id });
    Ok(())
}

pub async fn set_pinned(state: &AppState, user: &AuthUser, topic_id: Uuid, pinned: bool) -> Result<()> {
    set_flag(state, user, topic_id, "is_pinned", pinned, if pinned { "topic_pin" } else { "topic_unpin" }).await
}

pub async fn set_locked(state: &AppState, user: &AuthUser, topic_id: Uuid, locked: bool) -> Result<()> {
    set_flag(state, user, topic_id, "is_locked", locked, if locked { "topic_lock" } else { "topic_unlock" }).await
}

pub async fn move_topic(state: &AppState, user: &AuthUser, topic_id: Uuid, category_id: Uuid) -> Result<()> {
    if !user.is_mod() {
        return Err(Error::Forbidden);
    }
    let meta = topic_meta(&state.db, topic_id).await?;
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM forum_categories WHERE id = $1")
        .bind(category_id)
        .fetch_optional(&state.db)
        .await?;
    let name = name.ok_or(Error::NotFound("Kategorie"))?;
    sqlx::query("UPDATE forum_topics SET category_id = $2 WHERE id = $1")
        .bind(topic_id)
        .bind(category_id)
        .execute(&state.db)
        .await?;
    modlog::record(&state.db, Some(user.id), "topic_move", Some((SubjectType::ForumTopic, topic_id)), Some(meta.author_id), &format!("{} → {name}", meta.title)).await?;
    state
        .hub
        .publish(&Topic::ForumTopic(topic_id), &ServerMsg::ForumTopicUpdated { topic_id });
    Ok(())
}

pub async fn delete_topic(state: &AppState, user: &AuthUser, topic_id: Uuid) -> Result<()> {
    let meta = topic_meta(&state.db, topic_id).await?;
    let is_author = meta.author_id == user.id;
    if !user.is_mod() {
        let others: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM forum_posts WHERE topic_id = $1 AND author_id <> $2 AND NOT is_deleted)",
        )
        .bind(topic_id)
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
        if !is_author {
            return Err(Error::Forbidden);
        }
        if others {
            return Err(Error::invalid(
                "Themen mit Antworten anderer können nur von Moderatoren gelöscht werden.",
            ));
        }
    }
    sqlx::query("UPDATE forum_topics SET is_deleted = TRUE, deleted_by = $2 WHERE id = $1")
        .bind(topic_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    if !is_author {
        modlog::record(&state.db, Some(user.id), "delete_content", Some((SubjectType::ForumTopic, topic_id)), Some(meta.author_id), &meta.title).await?;
    }
    state
        .hub
        .publish(&Topic::ForumTopic(topic_id), &ServerMsg::ForumTopicUpdated { topic_id });
    Ok(())
}

pub async fn restore_topic(state: &AppState, user: &AuthUser, topic_id: Uuid) -> Result<()> {
    if !user.is_mod() {
        return Err(Error::Forbidden);
    }
    let meta = topic_meta(&state.db, topic_id).await?;
    sqlx::query("UPDATE forum_topics SET is_deleted = FALSE, deleted_by = NULL WHERE id = $1")
        .bind(topic_id)
        .execute(&state.db)
        .await?;
    modlog::record(&state.db, Some(user.id), "restore_content", Some((SubjectType::ForumTopic, topic_id)), Some(meta.author_id), &meta.title).await?;
    Ok(())
}

pub async fn set_subscription(db: &PgPool, user: &AuthUser, topic_id: Uuid, level: SubscriptionLevel) -> Result<()> {
    if !can_read_topic(db, Some(user), topic_id).await? {
        return Err(Error::NotFound("Thema"));
    }
    sqlx::query(
        "INSERT INTO forum_topic_subs (user_id, topic_id, level) VALUES ($1, $2, $3) \
         ON CONFLICT (user_id, topic_id) DO UPDATE SET level = EXCLUDED.level",
    )
    .bind(user.id)
    .bind(topic_id)
    .bind(level.as_str())
    .execute(db)
    .await?;
    Ok(())
}

/// Recent forum activity of a user (profile page), respecting visibility.
pub async fn user_activity(db: &PgPool, viewer: Option<&AuthUser>, user_id: Uuid, limit: i64) -> Result<Vec<ActivityItem>> {
    let mut qb = QueryBuilder::new(
        "SELECT p.id, p.topic_id, p.body_md, p.created_at, p.parent_post_id, t.title, t.first_post_id \
         FROM forum_posts p JOIN forum_topics t ON t.id = p.topic_id \
         JOIN forum_categories c ON c.id = t.category_id \
         WHERE NOT p.is_deleted AND NOT t.is_deleted AND p.author_id = ",
    );
    qb.push_bind(user_id);
    qb.push(" AND ");
    push_cat_visible(&mut qb, viewer);
    qb.push(" ORDER BY p.created_at DESC LIMIT ");
    qb.push_bind(limit);
    let rows = qb.build().fetch_all(db).await?;
    rows.iter()
        .map(|r| {
            let id: Uuid = r.try_get("id")?;
            let topic_id: Uuid = r.try_get("topic_id")?;
            let first: Option<Uuid> = r.try_get("first_post_id")?;
            let body: String = r.try_get("body_md")?;
            let title: String = r.try_get("title")?;
            let is_topic = first == Some(id);
            Ok(ActivityItem {
                icon: if is_topic { "message-square" } else { "message-circle" }.into(),
                label: if is_topic { "Neues Thema" } else { "Antwort" }.into(),
                title,
                excerpt: markdown::excerpt(&body, 220),
                link: format!("/forum/t/{topic_id}?post={id}"),
                created_at: r.try_get("created_at")?,
            })
        })
        .collect()
}

/// Rebuilds stored HTML of all posts (after renderer changes).
pub async fn rerender_all(db: &PgPool) -> Result<usize> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, body_md FROM forum_posts")
        .fetch_all(db)
        .await?;
    let n = rows.len();
    for (id, md) in rows {
        let html = markdown::render_mentions(db, &md).await?.html;
        sqlx::query("UPDATE forum_posts SET body_html = $2 WHERE id = $1")
            .bind(id)
            .bind(html)
            .execute(db)
            .await?;
    }
    Ok(n)
}
