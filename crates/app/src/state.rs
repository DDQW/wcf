//! Client-wide reactive state provided as context by `App`.

use leptos::prelude::*;
use shared::{AuthUser, Bootstrap, NavLink, PublicSiteInfo};

/// Who is logged in and how the site is configured.
#[derive(Clone, Copy)]
pub struct Session {
    pub user: RwSignal<Option<AuthUser>>,
    pub site: RwSignal<PublicSiteInfo>,
    pub nav: RwSignal<Vec<NavLink>>,
    pub unread: RwSignal<i64>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            user: RwSignal::new(None),
            site: RwSignal::new(PublicSiteInfo::default()),
            nav: RwSignal::new(Vec::new()),
            unread: RwSignal::new(0),
        }
    }

    pub fn apply(&self, b: Bootstrap) {
        self.user.set(b.user);
        self.site.set(b.site);
        self.nav.set(b.nav);
        self.unread.set(b.unread_notifications);
    }

    pub fn logged_in(&self) -> bool {
        self.user.with(|u| u.is_some())
    }

    pub fn is_mod(&self) -> bool {
        self.user.with(|u| u.as_ref().is_some_and(|u| u.is_mod()))
    }

    pub fn is_admin(&self) -> bool {
        self.user.with(|u| u.as_ref().is_some_and(|u| u.is_admin()))
    }

    pub fn user_id(&self) -> Option<uuid::Uuid> {
        self.user.with(|u| u.as_ref().map(|u| u.id))
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

pub fn use_session() -> Session {
    expect_context::<Session>()
}

/// Current time in milliseconds, ticking every 30 s on the client so
/// relative timestamps ("vor 5 Min.") stay fresh.
#[derive(Clone, Copy)]
pub struct Clock(pub RwSignal<f64>);

impl Clock {
    pub fn new() -> Self {
        Clock(RwSignal::new(now_ms()))
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

pub fn now_ms() -> f64 {
    #[cfg(feature = "hydrate")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(feature = "hydrate"))]
    {
        chrono::Utc::now().timestamp_millis() as f64
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub text: String,
    pub link: Option<String>,
}

/// Short-lived messages in the bottom corner.
#[derive(Clone, Copy)]
pub struct Toasts {
    pub items: RwSignal<Vec<Toast>>,
    next: RwSignal<u64>,
}

impl Toasts {
    pub fn new() -> Self {
        Self {
            items: RwSignal::new(Vec::new()),
            next: RwSignal::new(1),
        }
    }

    pub fn push(&self, kind: ToastKind, text: impl Into<String>, link: Option<String>) {
        let id = self.next.get_untracked();
        self.next.set(id + 1);
        self.items.update(|v| {
            v.push(Toast {
                id,
                kind,
                text: text.into(),
                link,
            });
            if v.len() > 5 {
                v.remove(0);
            }
        });
        #[cfg(feature = "hydrate")]
        {
            let items = self.items;
            set_timeout(
                move || items.update(|v| v.retain(|t| t.id != id)),
                std::time::Duration::from_millis(if kind == ToastKind::Error { 8000 } else { 5000 }),
            );
        }
    }

    pub fn success(&self, text: impl Into<String>) {
        self.push(ToastKind::Success, text, None);
    }

    pub fn error(&self, text: impl Into<String>) {
        self.push(ToastKind::Error, text, None);
    }

    pub fn info(&self, text: impl Into<String>, link: Option<String>) {
        self.push(ToastKind::Info, text, link);
    }

    pub fn dismiss(&self, id: u64) {
        self.items.update(|v| v.retain(|t| t.id != id));
    }
}

impl Default for Toasts {
    fn default() -> Self {
        Self::new()
    }
}

pub fn use_toasts() -> Toasts {
    expect_context::<Toasts>()
}
