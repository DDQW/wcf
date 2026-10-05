use chrono::{DateTime, Utc};
use leptos::prelude::*;
use shared::util::{hue_for, initials};
use shared::{Role, UserSummary};

use crate::components::icon::Icon;
use crate::error::AppError;
use crate::realtime::use_realtime;
use crate::state::{use_session, Clock};

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

#[component]
pub fn Avatar(
    user: UserSummary,
    #[prop(default = 36)] size: u32,
    /// Show a green dot when the user is online.
    #[prop(optional)]
    presence: bool,
) -> impl IntoView {
    let rt = use_realtime();
    let user_id = user.id;
    let style = format!("width:{size}px;height:{size}px;font-size:{}px", (size * 2 / 5).max(10));
    let inner = match user.avatar_url.clone() {
        Some(url) => view! {
            <img class="avatar" src=url alt=user.display_name.clone() style=style.clone() loading="lazy"/>
        }
        .into_any(),
        None => {
            let bg = format!("{style};background:hsl({},52%,46%)", hue_for(&user.id));
            view! {
                <span class="avatar avatar-initials" style=bg aria-hidden="true">
                    {initials(&user.display_name)}
                </span>
            }
            .into_any()
        }
    };
    view! {
        <span class="avatar-wrap">
            {inner}
            {presence.then(|| view! {
                <span class="presence-dot" class:online=move || rt.is_online(user_id)></span>
            })}
        </span>
    }
}

#[component]
pub fn RoleBadge(role: Role) -> impl IntoView {
    match role {
        Role::Admin => Some(view! { <span class="badge badge-admin">"Admin"</span> }),
        Role::Moderator => Some(view! { <span class="badge badge-mod">"Mod"</span> }),
        Role::Member => None,
    }
}

#[component]
pub fn UserLink(user: UserSummary, #[prop(optional)] badge: bool) -> impl IntoView {
    let role = user.role;
    view! {
        <a class="user-link" href=user.profile_url()>{user.display_name.clone()}</a>
        {badge.then(|| view! { <RoleBadge role=role/> })}
    }
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// "05.10.2026, 14:32" in the browser's local time zone.
pub fn local_datetime(ts: DateTime<Utc>) -> String {
    #[cfg(feature = "hydrate")]
    {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts.timestamp_millis() as f64));
        format!(
            "{:02}.{:02}.{}, {:02}:{:02}",
            d.get_date(),
            d.get_month() + 1,
            d.get_full_year(),
            d.get_hours(),
            d.get_minutes()
        )
    }
    #[cfg(not(feature = "hydrate"))]
    {
        ts.format("%d.%m.%Y, %H:%M UTC").to_string()
    }
}

/// "05.10.2026" in local time.
pub fn local_date(ts: DateTime<Utc>) -> String {
    #[cfg(feature = "hydrate")]
    {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts.timestamp_millis() as f64));
        format!("{:02}.{:02}.{}", d.get_date(), d.get_month() + 1, d.get_full_year())
    }
    #[cfg(not(feature = "hydrate"))]
    {
        ts.format("%d.%m.%Y").to_string()
    }
}

/// "14:32" in local time.
pub fn local_time(ts: DateTime<Utc>) -> String {
    #[cfg(feature = "hydrate")]
    {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts.timestamp_millis() as f64));
        format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
    }
    #[cfg(not(feature = "hydrate"))]
    {
        ts.format("%H:%M").to_string()
    }
}

pub fn relative_time(ts: DateTime<Utc>, now_ms: f64) -> String {
    let diff = (now_ms - ts.timestamp_millis() as f64) / 1000.0;
    let future = diff < 0.0;
    let secs = diff.abs();
    // Small clock differences between server and browser count as "now".
    let (n, unit_one, unit_many) = if secs < 60.0 {
        return "gerade eben".into();
    } else if secs < 3600.0 {
        ((secs / 60.0).round().max(1.0) as i64, "Min.", "Min.")
    } else if secs < 86_400.0 {
        ((secs / 3600.0).round() as i64, "Std.", "Std.")
    } else if secs < 7.0 * 86_400.0 {
        ((secs / 86_400.0).round() as i64, "Tag", "Tagen")
    } else {
        return local_date(ts);
    };
    let unit = if n == 1 { unit_one } else { unit_many };
    if future {
        let unit = if unit == "Tagen" { "Tagen" } else { unit };
        format!("in {n} {unit}")
    } else {
        format!("vor {n} {unit}")
    }
}

/// Relative timestamp ("vor 5 Min.") with the full local date as tooltip.
/// Renders a deterministic date during SSR/hydration and switches to the
/// local, relative text once mounted (avoids hydration mismatches).
#[component]
pub fn TimeAgo(ts: DateTime<Utc>, #[prop(optional)] absolute: bool) -> impl IntoView {
    let clock = expect_context::<Clock>();
    let mounted = RwSignal::new(false);
    Effect::new(move |_| mounted.set(true));
    let text = move || {
        if !mounted.get() {
            return ts.format("%d.%m.%Y").to_string();
        }
        if absolute {
            local_datetime(ts)
        } else {
            relative_time(ts, clock.0.get())
        }
    };
    let title = move || {
        if mounted.get() {
            local_datetime(ts)
        } else {
            ts.format("%d.%m.%Y %H:%M UTC").to_string()
        }
    };
    view! { <time datetime=ts.to_rfc3339() title=title>{text}</time> }
}

// ---------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------

#[component]
pub fn Spinner(#[prop(optional, into)] label: String) -> impl IntoView {
    view! {
        <div class="spinner-wrap" role="status">
            <span class="spinner"></span>
            {(!label.is_empty()).then(|| view! { <span class="muted">{label.clone()}</span> })}
        </div>
    }
}

#[component]
pub fn ErrorBox(error: AppError) -> impl IntoView {
    view! {
        <div class="alert alert-error">
            <Icon name="alert"/>
            <span>{error.to_string()}</span>
        </div>
    }
}

#[component]
pub fn EmptyState(
    #[prop(into)] icon: String,
    #[prop(into)] title: String,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="empty-state">
            <Icon name=icon size=40/>
            <p class="empty-title">{title}</p>
            {children.map(|c| c())}
        </div>
    }
}

/// Shows its children only to logged-in users, otherwise a login prompt.
#[component]
pub fn RequireLogin(children: ChildrenFn) -> impl IntoView {
    let session = use_session();
    view! {
        <Show
            when=move || session.logged_in()
            fallback=|| view! {
                <div class="card login-prompt">
                    <Icon name="lock" size=28/>
                    <p>"Für diesen Bereich musst du angemeldet sein."</p>
                    <div class="row gap">
                        <a class="btn btn-primary" href="/login">"Anmelden"</a>
                        <a class="btn" href="/register">"Registrieren"</a>
                    </div>
                </div>
            }
        >
            {children()}
        </Show>
    }
}

#[component]
pub fn PageHeader(
    #[prop(into)] title: String,
    #[prop(optional, into)] subtitle: String,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="page-header">
            <div>
                <h1>{title}</h1>
                {(!subtitle.is_empty()).then(|| view! { <p class="muted">{subtitle.clone()}</p> })}
            </div>
            <div class="page-actions">{children.map(|c| c())}</div>
        </div>
    }
}

/// Simple numbered pagination (0-based `page`).
#[component]
pub fn Pager(page: RwSignal<i64>, #[prop(into)] total: Signal<i64>, per_page: i64) -> impl IntoView {
    let pages = move || ((total.get() + per_page - 1) / per_page).max(1);
    view! {
        <Show when=move || { pages() > 1 }>
            <nav class="pager">
                <button class="btn btn-sm" disabled=move || page.get() == 0 on:click=move |_| page.update(|p| *p -= 1)>
                    <Icon name="chevron-left" size=16/>"Zurück"
                </button>
                <span class="muted">{move || format!("Seite {} von {}", page.get() + 1, pages())}</span>
                <button class="btn btn-sm" disabled=move || page.get() + 1 >= pages() on:click=move |_| page.update(|p| *p += 1)>
                    "Weiter"<Icon name="chevron-right" size=16/>
                </button>
            </nav>
        </Show>
    }
}

/// Copies text to the clipboard (best effort).
pub fn copy_to_clipboard(text: &str) {
    #[cfg(feature = "hydrate")]
    {
        let clipboard = window().navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = text;
}

/// Native confirm dialog; `false` during SSR.
pub fn confirm(message: &str) -> bool {
    #[cfg(feature = "hydrate")]
    {
        window().confirm_with_message(message).unwrap_or(false)
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = message;
        false
    }
}

/// Origin of the current page, e.g. `https://example.org`.
pub fn origin() -> String {
    #[cfg(feature = "hydrate")]
    {
        window().location().origin().unwrap_or_default()
    }
    #[cfg(not(feature = "hydrate"))]
    {
        String::new()
    }
}

/// Resolves after `ms` milliseconds (no-op on the server).
pub async fn sleep_ms(ms: u32) {
    #[cfg(feature = "hydrate")]
    {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = ms;
}
