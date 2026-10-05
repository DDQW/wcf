//! Subject lookups for modules added in later phases (chat, blogs, gallery,
//! files, CMS). Kept separate so `subjects.rs` stays small.

use shared::{AuthUser, SubjectType};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::subjects::SubjectInfo;

pub async fn lookup(
    _state: &AppState,
    _viewer: Option<&AuthUser>,
    _st: SubjectType,
    _id: Uuid,
) -> Result<Option<SubjectInfo>> {
    Ok(None)
}

pub async fn moderator_delete(
    _state: &AppState,
    _actor: &AuthUser,
    _st: SubjectType,
    _id: Uuid,
) -> Result<()> {
    Err(Error::invalid("Dieser Inhaltstyp kann hier nicht gelöscht werden."))
}
