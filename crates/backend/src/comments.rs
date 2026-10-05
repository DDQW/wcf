//! Generic threaded comments (blog posts, gallery images, CMS entries).

use std::collections::HashSet;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use shared::blog::CommentView;
use shared::{AuthUser, NotificationKind, SubjectType};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notifications::{self, NewNotification};
use crate::state::AppState;
use crate::users::opt_user;
use crate::{markdown, modlog, reactions, subjects};

/// Subject types that support comments.
fn commentable(st: SubjectType) -> bool {
    matches!(st, SubjectType::BlogPost | SubjectType::GalleryImage | SubjectType::CmsEntry)
}

async fn allowed(state: &AppState, st: SubjectType, id: Uuid) -> Result<bool> {
    match st {
        SubjectType::BlogPost => crate::blogs::comments_allowed(&state.db, id).await,
        other => crate::extensions::comments_allowed(state, other, id).await,
    }
}

/// Owner of the commented thing (may moderate its comments).
async fn subject_owner(state: &AppState, st: SubjectType, id: Uuid) -> Result<Option<Uuid>> {
    match st {
        SubjectType::BlogPost => crate::blogs::owner_of(&state.db, id).await,
        other => crate::extensions::subject_owner(state, other, id).await,
    }
}

pub async fn list(state: &AppState, viewer: Option<&AuthUser>, st: SubjectType, id: Uuid) -> Result<Vec<CommentView>> {
    if !commentable(st) {
        return Err(Error::invalid("Hier gibt es keine Kommentare."));
    }
    subjects::lookup(state, viewer, st, id)
        .await?
        .ok_or(Error::NotFound("Inhalt"))?;
    let owner = subject_owner(state, st, id).await?;
    let rows = sqlx::query(&format!(
        "SELECT c.id, c.parent_id, c.body_html, c.created_at, c.edited_at, c.deleted_at, c.author_id AS author_ref, {} \
         FROM comments c LEFT JOIN users u ON u.id = c.author_id \
         WHERE c.subject_type = $1 AND c.subject_id = $2 ORDER BY c.created_at LIMIT 2000",
        crate::users::user_cols("u", "author")
    ))
    .bind(st.as_str())
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let ids: Vec<Uuid> = rows.iter().filter_map(|r| r.try_get("id").ok()).collect();
    let mut counts = reactions::counts_for(&state.db, SubjectType::Comment, &ids, viewer.map(|v| v.id)).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let cid: Uuid = r.try_get("id")?;
        let deleted_at: Option<DateTime<Utc>> = r.try_get("deleted_at")?;
        let author_ref: Option<Uuid> = r.try_get("author_ref")?;
        let is_author = viewer.is_some_and(|v| Some(v.id) == author_ref);
        let can_delete = is_author || viewer.is_some_and(|v| v.is_mod() || Some(v.id) == owner);
        out.push(CommentView {
            id: cid,
            author: opt_user(r, "author")?,
            parent_id: r.try_get("parent_id")?,
            body_html: if deleted_at.is_some() { String::new() } else { r.try_get("body_html")? },
            created_at: r.try_get("created_at")?,
            edited_at: r.try_get("edited_at")?,
            deleted: deleted_at.is_some(),
            reactions: counts.remove(&cid).unwrap_or_default(),
            can_edit: is_author && deleted_at.is_none(),
            can_delete: can_delete && deleted_at.is_none(),
        });
    }
    Ok(out)
}

pub async fn create(
    state: &AppState,
    user: &AuthUser,
    st: SubjectType,
    id: Uuid,
    parent: Option<Uuid>,
    body: &str,
) -> Result<CommentView> {
    if !commentable(st) {
        return Err(Error::invalid("Hier gibt es keine Kommentare."));
    }
    state
        .limiter
        .hit(&format!("comment:{}", user.id), 10, StdDuration::from_secs(60))?;
    let info = subjects::lookup(state, Some(user), st, id)
        .await?
        .ok_or(Error::NotFound("Inhalt"))?;
    if !allowed(state, st, id).await? {
        return Err(Error::invalid("Kommentare sind hier deaktiviert."));
    }
    let body = body.trim();
    if body.is_empty() || body.chars().count() > 10_000 {
        return Err(Error::invalid("Der Kommentar muss 1–10.000 Zeichen lang sein."));
    }
    let parent_author = match parent {
        Some(pid) => {
            let r: Option<(Option<Uuid>, String, Uuid)> =
                sqlx::query_as("SELECT author_id, subject_type, subject_id FROM comments WHERE id = $1")
                    .bind(pid)
                    .fetch_optional(&state.db)
                    .await?;
            let (author, pst, psid) = r.ok_or(Error::NotFound("Kommentar"))?;
            if pst != st.as_str() || psid != id {
                return Err(Error::invalid("Ungültige Antwort."));
            }
            author
        }
        None => None,
    };
    let rendered = markdown::render_mentions(&state.db, body).await?;
    let cid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO comments (id, subject_type, subject_id, author_id, parent_id, body_md, body_html) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(cid)
    .bind(st.as_str())
    .bind(id)
    .bind(user.id)
    .bind(parent)
    .bind(body)
    .bind(&rendered.html)
    .execute(&state.db)
    .await?;

    let link = info.link.clone().map(|l| {
        let sep = if l.contains('?') { '&' } else { '?' };
        format!("{l}{sep}comment={cid}")
    }).unwrap_or_default();
    let excerpt = markdown::excerpt(body, 160);
    let mut notified: HashSet<Uuid> = HashSet::from([user.id]);
    if let Some(pa) = parent_author {
        if notified.insert(pa) {
            notifications::notify(state, NewNotification {
                user_id: pa,
                kind: NotificationKind::Reply,
                actor_id: Some(user.id),
                title: format!("{} hat auf deinen Kommentar zu „{}“ geantwortet", user.display_name, info.title),
                body: excerpt.clone(),
                link: link.clone(),
                subject: Some((SubjectType::Comment, cid)),
            }).await;
        }
    }
    if let Some(owner) = info.author_id {
        if notified.insert(owner) {
            notifications::notify(state, NewNotification {
                user_id: owner,
                kind: NotificationKind::Comment,
                actor_id: Some(user.id),
                title: format!("{} hat „{}“ kommentiert", user.display_name, info.title),
                body: excerpt.clone(),
                link: link.clone(),
                subject: Some((SubjectType::Comment, cid)),
            }).await;
        }
    }
    for uid in rendered.mentioned {
        if notified.insert(uid) {
            notifications::notify(state, NewNotification {
                user_id: uid,
                kind: NotificationKind::Mention,
                actor_id: Some(user.id),
                title: format!("{} hat dich in einem Kommentar zu „{}“ erwähnt", user.display_name, info.title),
                body: excerpt.clone(),
                link: link.clone(),
                subject: Some((SubjectType::Comment, cid)),
            }).await;
        }
    }
    Ok(CommentView {
        id: cid,
        author: Some(user.summary()),
        parent_id: parent,
        body_html: rendered.html,
        created_at: Utc::now(),
        edited_at: None,
        deleted: false,
        reactions: Vec::new(),
        can_edit: true,
        can_delete: true,
    })
}

struct Meta {
    st: SubjectType,
    subject_id: Uuid,
    author: Option<Uuid>,
    body_md: String,
}

async fn meta(state: &AppState, id: Uuid) -> Result<Meta> {
    let r = sqlx::query("SELECT subject_type, subject_id, author_id, body_md FROM comments WHERE id = $1 AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(Error::NotFound("Kommentar"))?;
    let st: String = r.try_get("subject_type")?;
    Ok(Meta {
        st: SubjectType::parse(&st).ok_or(Error::NotFound("Kommentar"))?,
        subject_id: r.try_get("subject_id")?,
        author: r.try_get("author_id")?,
        body_md: r.try_get("body_md")?,
    })
}

pub async fn source(state: &AppState, user: &AuthUser, id: Uuid) -> Result<String> {
    let m = meta(state, id).await?;
    if m.author != Some(user.id) {
        return Err(Error::Forbidden);
    }
    Ok(m.body_md)
}

pub async fn edit(state: &AppState, user: &AuthUser, id: Uuid, body: &str) -> Result<String> {
    let m = meta(state, id).await?;
    if m.author != Some(user.id) {
        return Err(Error::Forbidden);
    }
    let body = body.trim();
    if body.is_empty() || body.chars().count() > 10_000 {
        return Err(Error::invalid("Der Kommentar muss 1–10.000 Zeichen lang sein."));
    }
    let html = markdown::render_mentions(&state.db, body).await?.html;
    sqlx::query("UPDATE comments SET body_md = $2, body_html = $3, edited_at = now() WHERE id = $1")
        .bind(id)
        .bind(body)
        .bind(&html)
        .execute(&state.db)
        .await?;
    Ok(html)
}

pub async fn delete(state: &AppState, user: &AuthUser, id: Uuid) -> Result<()> {
    let m = meta(state, id).await?;
    let owner = subject_owner(state, m.st, m.subject_id).await?;
    let is_author = m.author == Some(user.id);
    if !(is_author || user.is_mod() || owner == Some(user.id)) {
        return Err(Error::Forbidden);
    }
    sqlx::query("UPDATE comments SET deleted_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if !is_author {
        modlog::record(&state.db, Some(user.id), "delete_content", Some((SubjectType::Comment, id)), m.author, &markdown::excerpt(&m.body_md, 120)).await?;
    }
    Ok(())
}

/// Subject info for reactions/reports on comments.
pub async fn subject_info(state: &AppState, viewer: Option<&AuthUser>, id: Uuid) -> Result<Option<subjects::SubjectInfo>> {
    let Ok(m) = meta(state, id).await else { return Ok(None) };
    let Some(parent) = Box::pin(subjects::lookup(state, viewer, m.st, m.subject_id)).await? else {
        return Ok(None);
    };
    Ok(Some(subjects::SubjectInfo {
        author_id: m.author,
        title: format!("Kommentar zu „{}“", parent.title),
        preview: markdown::excerpt(&m.body_md, 200),
        link: parent.link.map(|l| {
            let sep = if l.contains('?') { '&' } else { '?' };
            format!("{l}{sep}comment={id}")
        }),
    }))
}
