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
        _ => Ok(None),
    }
}

/// Module-specific read access to files that are neither public nor
/// members-visible (e.g. attachments in private chat channels).
pub async fn file_access(_state: &AppState, _rec: &FileRecord, _viewer: Option<&AuthUser>) -> bool {
    false
}

/// Validates the `context_id` of an upload (e.g. chat channel membership).
pub async fn check_upload_context(_state: &AppState, _user: &AuthUser, _meta: &UploadMeta) -> Result<()> {
    Ok(())
}

pub async fn moderator_delete(
    _state: &AppState,
    _actor: &AuthUser,
    st: SubjectType,
    id: Uuid,
) -> Result<()> {
    match st {
        SubjectType::File => files::trash_files(_state, _actor, &[id]).await,
        _ => Err(Error::invalid("Dieser Inhaltstyp kann hier nicht gelöscht werden.")),
    }
}
