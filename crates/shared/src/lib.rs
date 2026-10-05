//! Types shared between the server and the WASM client: DTOs, the realtime
//! protocol, validation and formatting helpers. No server-only deps here.

pub mod admin;
pub mod auth;
pub mod forum;
pub mod moderation;
pub mod notification;
pub mod pagination;
pub mod reaction;
pub mod search;
pub mod user;
pub mod util;
pub mod ws;

pub use admin::{AdminStats, AdminUserRow, ModuleToggles, PublicSiteInfo, SiteSettings};
pub use auth::{Bootstrap, CommunityStats, HomeData, HomeExtra, NavLink};
pub use notification::{NotificationKind, NotificationView};
pub use pagination::{Cursor, Page, DEFAULT_PAGE_SIZE};
pub use reaction::{ReactionCount, SubjectType, REACTIONS};
pub use user::{AuthUser, Role, Theme, UserSummary};
