use leptos::server_fn::codec::Json;
use leptos::prelude::*;
use shared::moderation::{ModLogPage, ReportPage, ReportReason, ReportStatus};
use shared::{ReactionCount, Role, SubjectType, UserSummary};
use uuid::Uuid;

use crate::error::AppError;

#[server(input = Json)]
pub async fn toggle_reaction(
    subject_type: SubjectType,
    subject_id: Uuid,
    emoji: String,
) -> Result<Vec<ReactionCount>, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    Ok(backend::reactions::toggle(&st, &user, subject_type, subject_id, &emoji).await?)
}

#[server(input = Json)]
pub async fn reaction_users(
    subject_type: SubjectType,
    subject_id: Uuid,
    emoji: String,
) -> Result<Vec<UserSummary>, AppError> {
    let st = crate::ssr::state()?;
    Ok(backend::reactions::reactors(&st.db, subject_type, subject_id, &emoji).await?)
}

#[server(input = Json)]
pub async fn report_content(
    subject_type: SubjectType,
    subject_id: Uuid,
    reason: ReportReason,
    details: String,
) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_user().await?;
    backend::reports::create(&st, &user, subject_type, subject_id, reason, &details).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn list_reports(status: ReportStatus, page: i64) -> Result<ReportPage, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_mod().await?;
    let per_page = 20;
    let (items, total) = backend::reports::list(&st, &user, status, page.max(0) * per_page, per_page).await?;
    Ok(ReportPage { items, total })
}

#[server(input = Json)]
pub async fn resolve_report(
    report_id: Uuid,
    status: ReportStatus,
    note: String,
    delete_content: bool,
) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_mod().await?;
    backend::reports::resolve(&st, &user, report_id, status, &note, delete_content).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn mod_log(page: i64) -> Result<ModLogPage, AppError> {
    let st = crate::ssr::state()?;
    crate::ssr::require_mod().await?;
    let per_page = 50;
    let (items, total) = backend::modlog::list(&st.db, page.max(0) * per_page, per_page).await?;
    Ok(ModLogPage { items, total })
}

#[server(input = Json)]
pub async fn ban_user(user_id: Uuid, days: Option<i64>, reason: String) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_mod().await?;
    backend::users::ban(&st, &user, user_id, days, &reason).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn unban_user(user_id: Uuid) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_mod().await?;
    backend::users::unban(&st, &user, user_id).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn set_user_role(user_id: Uuid, role: Role) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_admin().await?;
    backend::users::set_role(&st, &user, user_id, role).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn set_user_active(user_id: Uuid, active: bool) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_admin().await?;
    backend::users::set_active(&st, &user, user_id, active).await?;
    Ok(())
}

#[server(input = Json)]
pub async fn admin_reset_link(user_id: Uuid) -> Result<String, AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_admin().await?;
    Ok(backend::auth::admin_reset_link(&st, &user, user_id).await?)
}

#[server(input = Json)]
pub async fn set_user_quota(user_id: Uuid, quota_mb: Option<i64>) -> Result<(), AppError> {
    let st = crate::ssr::state()?;
    let user = crate::ssr::require_admin().await?;
    backend::users::set_quota(&st, &user, user_id, quota_mb).await?;
    Ok(())
}
