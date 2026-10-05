//! Subject lookups for modules added in later phases (chat, blogs, gallery,
//! files, CMS). Kept separate so `subjects.rs` stays small.

use shared::{AuthUser, SubjectType};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::files::{self, FileRecord, UploadMeta};
use crate::state::AppState;
use crate::subjects::SubjectInfo;

pub async fn lookup(
    state: &AppState,
    viewer: Option<&AuthUser>,
    st: SubjectType,
    id: Uuid,
) -> Result<Option<SubjectInfo>> {
    match st {
        SubjectType::File => {
            let Some(rec) = files::get_record(&state.db, id).await? else { return Ok(None) };
            if !files::can_view(state, &rec, viewer).await {
                return Ok(None);
            }
            Ok(Some(SubjectInfo {
                author_id: Some(rec.owner_id),
                title: rec.view.name.clone(),
                preview: format!("{} ({})", rec.view.name, shared::util::format_bytes(rec.view.size_bytes)),
                link: Some(rec.view.url()),
            }))
        }
        SubjectType::ChatMessage => crate::chat::subject_info(&state.db, viewer, id).await,
        SubjectType::BlogPost => crate::blogs::subject_info(&state.db, viewer, id).await,
        SubjectType::Comment => crate::comments::subject_info(state, viewer, id).await,
        _ => Ok(None),
    }
}

/// Module-specific read access to files that are neither public nor
/// members-visible (e.g. attachments in private chat channels).
pub async fn file_access(state: &AppState, rec: &FileRecord, viewer: Option<&AuthUser>) -> bool {
    match (rec.view.purpose, rec.context_id) {
        (shared::files::FilePurpose::Chat, Some(channel)) => {
            crate::chat::attachment_access(&state.db, rec.view.id, channel, viewer).await
        }
        _ => false,
    }
}

/// Validates the `context_id` of an upload (e.g. chat channel membership)
/// and may force a visibility for it.
pub async fn check_upload_context(
    state: &AppState,
    user: &AuthUser,
    meta: &UploadMeta,
) -> Result<Option<shared::files::Visibility>> {
    match meta.purpose {
        shared::files::FilePurpose::Chat => {
            let channel = meta
                .context_id
                .ok_or_else(|| Error::invalid("Kanal für den Anhang fehlt."))?;
            Ok(Some(crate::chat::upload_visibility(&state.db, user, channel).await?))
        }
        _ => Ok(None),
    }
}

/// Runs after a reaction changed (live updates for chat messages).
pub async fn after_reaction(state: &AppState, st: SubjectType, id: Uuid) {
    if st == SubjectType::ChatMessage {
        let _ = crate::chat::broadcast_reactions(state, id).await;
    }
}

pub async fn moderator_delete(
    _state: &AppState,
    _actor: &AuthUser,
    st: SubjectType,
    id: Uuid,
) -> Result<()> {
    match st {
        SubjectType::File => files::trash_files(_state, _actor, &[id]).await,
        SubjectType::ChatMessage => crate::chat::delete(_state, _actor, id).await,
        SubjectType::BlogPost => crate::blogs::delete_post(_state, _actor, id).await,
        SubjectType::Comment => crate::comments::delete(_state, _actor, id).await,
        _ => Err(Error::invalid("Dieser Inhaltstyp kann hier nicht gelöscht werden.")),
    }
}

/// Whether comments are enabled on subjects of later modules.
pub async fn comments_allowed(_state: &AppState, _st: SubjectType, _id: Uuid) -> Result<bool> {
    Ok(false)
}

/// Owner of a commentable subject of later modules.
pub async fn subject_owner(_state: &AppState, _st: SubjectType, _id: Uuid) -> Result<Option<Uuid>> {
    Ok(None)
}
