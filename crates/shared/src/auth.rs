use serde::{Deserialize, Serialize};

use crate::admin::PublicSiteInfo;
use crate::forum::TopicListItem;
use crate::user::{AuthUser, UserSummary};

/// Initial payload for the app shell: who is logged in and how the site is
/// configured. Loaded once per page load in a single round trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bootstrap {
    pub user: Option<AuthUser>,
    pub site: PublicSiteInfo,
    pub unread_notifications: i64,
    pub nav: Vec<NavLink>,
}

/// Extra navigation entry (managed through the CMS menu editor).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommunityStats {
    pub members: i64,
    pub topics: i64,
    pub posts: i64,
    pub blog_posts: i64,
    pub images: i64,
}

/// Everything the start page shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HomeData {
    pub latest_topics: Vec<TopicListItem>,
    pub online: Vec<UserSummary>,
    pub newest_members: Vec<UserSummary>,
    pub stats: CommunityStats,
    /// Filled by later modules (blogs, gallery, watch rooms …).
    pub extra: HomeExtra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HomeExtra {}
