use leptos::prelude::*;
use leptos_router::hooks::{use_location, use_navigate};
use shared::ws::ServerMsg;
use shared::{NotificationView, Theme};

use crate::api::notifications::{list_notifications, mark_all_notifications_read};
use crate::components::common::{Avatar, TimeAgo};
use crate::components::icon::Icon;
use crate::realtime::use_realtime;
use crate::routes::AppRoutes;
use crate::state::{use_session, use_toasts, ToastKind};

/// Sidebar + top bar + routed content.
#[component]
pub fn Shell() -> impl IntoView {
    crate::pages::chat::provide_chat_badge();
    let sidebar_open = RwSignal::new(false);
    let location = use_location();
    Effect::new(move |_| {
        location.pathname.track();
        sidebar_open.set(false);
    });
    let rt = use_realtime();
    let toasts = use_toasts();
    rt.on_message(move |msg| {
        if let ServerMsg::Notification { notification, .. } = msg {
            let link = (!notification.link.is_empty()).then(|| notification.link.clone());
            toasts.info(notification.title.clone(), link);
        }
    });

    view! {
        <div class="app" class:sidebar-open=move || sidebar_open.get()>
            <Sidebar/>
            <div class="sidebar-backdrop" on:click=move |_| sidebar_open.set(false)></div>
            <div class="main">
                <Topbar sidebar_open=sidebar_open/>
                <main class="content" id="content">
                    <AppRoutes/>
                </main>
            </div>
            <ToastHost/>
            <crate::pages::calls::IncomingCallHost/>
        </div>
    }
}

#[component]
pub fn NavItem(
    #[prop(into)] href: String,
    #[prop(into)] icon: String,
    #[prop(into)] label: String,
    #[prop(optional)] exact: bool,
    #[prop(optional, into)] badge: Option<Signal<i64>>,
) -> impl IntoView {
    let location = use_location();
    let target = href.clone();
    let active = move || {
        let path = location.pathname.get();
        if exact {
            path == target
        } else {
            path == target || path.starts_with(&format!("{target}/"))
        }
    };
    view! {
        <a class="nav-item" class:active=active href=href>
            <Icon name=icon size=19/>
            <span class="nav-label">{label}</span>
            {badge.map(|b| view! {
                <Show when=move || { b.get() > 0 }>
                    <span class="nav-badge">{move || b.get()}</span>
                </Show>
            })}
        </a>
    }
}

#[component]
fn Sidebar() -> impl IntoView {
    let session = use_session();
    let modules = move || session.site.with(|s| s.modules.clone());
    view! {
        <aside class="sidebar">
            <a class="brand" href="/">
                <span class="brand-mark">
                    <Icon name="zap" size=18/>
                </span>
                <span class="brand-name">{move || session.site.with(|s| s.site_name.clone())}</span>
            </a>
            <nav class="nav">
                <NavItem href="/" icon="home" label="Start" exact=true/>
                <Show when=move || modules().forum>
                    <NavItem href="/forum" icon="message-square" label="Forum"/>
                </Show>
                <NavItem href="/members" icon="users" label="Mitglieder"/>
                {crate::routes::module_nav()}
                <For
                    each=move || session.nav.get()
                    key=|l| l.url.clone()
                    children=|l| view! { <NavItem href=l.url.clone() icon="file-text" label=l.label.clone()/> }
                />
            </nav>
            <div class="nav nav-bottom">
                <Show when=move || session.is_mod()>
                    <NavItem href="/admin" icon="shield" label="Verwaltung"/>
                </Show>
            </div>
        </aside>
    }
}

#[component]
fn Topbar(sidebar_open: RwSignal<bool>) -> impl IntoView {
    let session = use_session();
    let navigate = use_navigate();
    let query = RwSignal::new(String::new());
    let on_search = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let q = query.get_untracked();
        if !q.trim().is_empty() {
            navigate(&format!("/search?q={}", urlencode(q.trim())), Default::default());
        }
    };
    view! {
        <header class="topbar">
            <button class="icon-btn menu-btn" aria-label="Menü" on:click=move |_| sidebar_open.update(|o| *o = !*o)>
                <Icon name="menu"/>
            </button>
            <form class="search-box" role="search" on:submit=on_search>
                <Icon name="search" size=17/>
                <input type="search" placeholder="Suchen …" aria-label="Suchen" bind:value=query/>
            </form>
            <div class="topbar-actions">
                <ThemeToggle/>
                <Show
                    when=move || session.logged_in()
                    fallback=|| view! {
                        <a class="btn btn-ghost" href="/login">"Anmelden"</a>
                        <a class="btn btn-primary" href="/register">"Registrieren"</a>
                    }
                >
                    <NotificationBell/>
                    <UserMenu/>
                </Show>
            </div>
        </header>
    }
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Cycles system → light → dark. Logged-in users persist the choice.
#[component]
fn ThemeToggle() -> impl IntoView {
    let session = use_session();
    let theme = expect_context::<RwSignal<Theme>>();
    let next = move || match theme.get() {
        Theme::System => Theme::Light,
        Theme::Light => Theme::Dark,
        Theme::Dark => Theme::System,
    };
    let toggle = move |_| {
        let t = next();
        theme.set(t);
        crate::app::store_local_theme(t);
        if session.logged_in() {
            leptos::task::spawn_local(async move {
                let _ = crate::api::users::set_theme(t).await;
            });
        }
    };
    view! {
        <button class="icon-btn" title=move || format!("Darstellung: {}", theme.get().label()) on:click=toggle>
            {move || match theme.get() {
                Theme::Dark => view! { <Icon name="moon"/> }.into_any(),
                Theme::Light => view! { <Icon name="sun"/> }.into_any(),
                Theme::System => view! { <Icon name="monitor"/> }.into_any(),
            }}
        </button>
    }
}

#[component]
fn NotificationBell() -> impl IntoView {
    let session = use_session();
    let open = RwSignal::new(false);
    let items = RwSignal::new(Vec::<NotificationView>::new());
    let loading = RwSignal::new(false);

    let load = move || {
        loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(page) = list_notifications(None, false).await {
                items.set(page.items.into_iter().take(8).collect());
            }
            loading.set(false);
        });
    };
    #[cfg(feature = "hydrate")]
    {
        let handle = window_event_listener(leptos::ev::click, move |_| {
            if open.get_untracked() {
                open.set(false);
            }
        });
        on_cleanup(move || handle.remove());
    }
    let toggle = move |_| {
        let now = !open.get_untracked();
        open.set(now);
        if now {
            load();
        }
    };
    let mark_all = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(unread) = mark_all_notifications_read().await {
                session.unread.set(unread);
                items.update(|v| v.iter_mut().for_each(|n| n.is_read = true));
            }
        });
    };

    view! {
        <div class="dropdown bell" on:click=|ev| ev.stop_propagation()>
            <button class="icon-btn" aria-label="Benachrichtigungen" on:click=toggle>
                <Icon name="bell"/>
                <Show when=move || { session.unread.get() > 0 }>
                    <span class="bell-badge">{move || {
                        let n = session.unread.get();
                        if n > 99 { "99+".to_string() } else { n.to_string() }
                    }}</span>
                </Show>
            </button>
            <Show when=move || open.get()>
                <div class="dropdown-menu align-right notif-menu">
                    <div class="notif-head">
                        <strong>"Benachrichtigungen"</strong>
                        <button class="link-btn" on:click=mark_all>"Alle gelesen"</button>
                    </div>
                    {move || if loading.get() && items.with(|i| i.is_empty()) {
                        view! { <p class="muted pad">"Lade …"</p> }.into_any()
                    } else if items.with(|i| i.is_empty()) {
                        view! { <p class="muted pad">"Keine Benachrichtigungen."</p> }.into_any()
                    } else {
                        view! {
                            <ul class="notif-list">
                                <For each=move || items.get() key=|n| (n.id, n.is_read) children=move |n| view! {
                                    <NotificationRow n=n on_open=Callback::new(move |_| open.set(false))/>
                                }/>
                            </ul>
                        }.into_any()
                    }}
                    <a class="notif-all" href="/notifications" on:click=move |_| open.set(false)>"Alle anzeigen"</a>
                </div>
            </Show>
        </div>
    }
}

#[component]
pub fn NotificationRow(n: NotificationView, #[prop(optional)] on_open: Option<Callback<()>>) -> impl IntoView {
    let session = use_session();
    let id = n.id;
    let unread = n.is_read;
    let href = if n.link.is_empty() { "/notifications".to_string() } else { n.link.clone() };
    let mark = move |_| {
        if !unread {
            leptos::task::spawn_local(async move {
                if let Ok(u) = crate::api::notifications::mark_notifications_read(vec![id]).await {
                    session.unread.set(u);
                }
            });
        }
        if let Some(cb) = on_open {
            cb.run(());
        }
    };
    view! {
        <li class="notif" class:unread=!n.is_read>
            <a href=href on:click=mark>
                <span class="notif-icon">
                    {match n.actor.clone() {
                        Some(a) => view! { <Avatar user=a size=32/> }.into_any(),
                        None => view! { <Icon name=n.kind.icon() size=20/> }.into_any(),
                    }}
                </span>
                <span class="notif-text">
                    <span class="notif-title">{n.title.clone()}</span>
                    {(!n.body.is_empty()).then(|| view! { <span class="notif-body">{n.body.clone()}</span> })}
                    <span class="notif-time"><TimeAgo ts=n.created_at/></span>
                </span>
            </a>
        </li>
    }
}

#[component]
fn UserMenu() -> impl IntoView {
    let session = use_session();
    let rt = use_realtime();
    let navigate = use_navigate();
    let open = RwSignal::new(false);
    #[cfg(feature = "hydrate")]
    {
        let handle = window_event_listener(leptos::ev::click, move |_| {
            if open.get_untracked() {
                open.set(false);
            }
        });
        on_cleanup(move || handle.remove());
    }
    let logout = move |_| {
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            let _ = crate::api::auth::logout().await;
            session.user.set(None);
            session.unread.set(0);
            rt.reconnect();
            navigate("/", Default::default());
        });
    };
    view! {
        <div class="dropdown" on:click=|ev| ev.stop_propagation()>
            <button class="user-btn" aria-label="Benutzermenü" on:click=move |_| open.update(|o| *o = !*o)>
                {move || session.user.get().map(|u| view! { <Avatar user=u.summary() size=32/> })}
            </button>
            <Show when=move || open.get()>
                <div class="dropdown-menu align-right" on:click=move |_| open.set(false)>
                    {move || session.user.get().map(|u| view! {
                        <div class="menu-head">
                            <strong>{u.display_name.clone()}</strong>
                            <span class="muted small">"@"{u.username.clone()}</span>
                        </div>
                        <a class="menu-item" href=format!("/u/{}", u.username)><Icon name="user" size=16/>"Mein Profil"</a>
                    })}
                    {crate::routes::user_menu_items()}
                    <a class="menu-item" href="/notifications"><Icon name="bell" size=16/>"Benachrichtigungen"</a>
                    <a class="menu-item" href="/settings"><Icon name="sliders" size=16/>"Einstellungen"</a>
                    <Show when=move || session.is_mod()>
                        <a class="menu-item" href="/admin"><Icon name="shield" size=16/>"Verwaltung"</a>
                    </Show>
                    <hr/>
                    <button class="menu-item" on:click=logout.clone()><Icon name="log-out" size=16/>"Abmelden"</button>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn ToastHost() -> impl IntoView {
    let toasts = use_toasts();
    view! {
        <div class="toasts" aria-live="polite">
            <For each=move || toasts.items.get() key=|t| t.id children=move |t| {
                let id = t.id;
                let class = match t.kind {
                    ToastKind::Info => "toast",
                    ToastKind::Success => "toast toast-success",
                    ToastKind::Error => "toast toast-error",
                };
                let icon = match t.kind {
                    ToastKind::Info => "bell",
                    ToastKind::Success => "check-circle",
                    ToastKind::Error => "alert",
                };
                view! {
                    <div class=class>
                        <Icon name=icon size=18/>
                        {match t.link.clone() {
                            Some(href) => view! {
                                <a class="toast-text" href=href on:click=move |_| toasts.dismiss(id)>{t.text.clone()}</a>
                            }.into_any(),
                            None => view! { <span class="toast-text">{t.text.clone()}</span> }.into_any(),
                        }}
                        <button class="icon-btn" aria-label="Schließen" on:click=move |_| toasts.dismiss(id)>
                            <Icon name="x" size=14/>
                        </button>
                    </div>
                }
            }/>
        </div>
    }
}
