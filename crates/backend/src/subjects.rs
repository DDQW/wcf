//! Uniform access to "things" (posts, messages, images …) for the generic
//! features that work across modules: reactions, reports and comments.

use shared::{AuthUser, SubjectType};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::forum;
use crate::markdown;
use crate::state::AppState;

pub struct SubjectInfo {
    pub author_id: Option<Uuid>,
    /// Short human description, e.g. the topic title.
    pub title: String,
    pub preview: String,
    pub link: Option<String>,
}

/// Looks a subject up and checks that `viewer` may see it. Returns `None`
/// for unknown/invisible subjects.
pub async fn lookup(
    state: &AppState,
    viewer: Option<&AuthUser>,
    st: SubjectType,
    id: Uuid,
) -> Result<Option<SubjectInfo>> {
    let db = &state.db;
    match st {
        SubjectType::ForumPost => {
            let row = sqlx::query(
                "SELECT p.author_id, p.body_md, p.topic_id, t.title, p.is_deleted \
                 FROM forum_posts p JOIN forum_topics t ON t.id = p.topic_id WHERE p.id = $1",
            )
            .bind(id)
            .fetch_optional(db)
            .await?;
            let Some(r) = row else { return Ok(None) };
            let topic_id: Uuid = r.try_get("topic_id")?;
            if !forum::can_read_topic(db, viewer, topic_id).await? {
                return Ok(None);
            }
            let body: String = r.try_get("body_md")?;
            Ok(Some(SubjectInfo {
                author_id: r.try_get("author_id")?,
                title: r.try_get("title")?,
                preview: markdown::excerpt(&body, 200),
                link: Some(format!("/forum/t/{topic_id}?post={id}")),
            }))
        }
        SubjectType::ForumTopic => {
            if !forum::can_read_topic(db, viewer, id).await? {
                return Ok(None);
            }
            let row = sqlx::query("SELECT author_id, title FROM forum_topics WHERE id = $1")
                .bind(id)
                .fetch_optional(db)
                .await?;
            let Some(r) = row else { return Ok(None) };
            let title: String = r.try_get("title")?;
            Ok(Some(SubjectInfo {
                author_id: r.try_get("author_id")?,
                preview: title.clone(),
                title,
                link: Some(format!("/forum/t/{id}")),
            }))
        }
        SubjectType::User => {
            let row = sqlx::query(
                "SELECT username::text AS username, display_name, bio_md FROM users WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(db)
            .await?;
            let Some(r) = row else { return Ok(None) };
            let username: String = r.try_get("username")?;
            let bio: String = r.try_get("bio_md")?;
            Ok(Some(SubjectInfo {
                author_id: Some(id),
                title: r.try_get("display_name")?,
                preview: markdown::excerpt(&bio, 200),
                link: Some(format!("/u/{username}")),
            }))
        }
        other => crate::extensions::lookup(state, viewer, other, id).await,
    }
}

/// Deletes a subject on behalf of a moderator (used from the report queue).
pub async fn moderator_delete(state: &AppState, actor: &AuthUser, st: SubjectType, id: Uuid) -> Result<()> {
    if !actor.is_mod() {
        return Err(Error::Forbidden);
    }
    match st {
        SubjectType::ForumPost => forum::delete_post(state, actor, id).await,
        SubjectType::ForumTopic => forum::delete_topic(state, actor, id).await,
        SubjectType::User => Err(Error::invalid(
            "Benutzer können nicht gelöscht werden – bitte sperren.",
        )),
        other => crate::extensions::moderator_delete(state, actor, other, id).await,
    }
}
