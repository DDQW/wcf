mod cms;
mod forum;
mod log;
mod reports;
mod settings;
mod users;

pub use cms::{AdminCms, AdminCmsType, AdminMenu};
pub use forum::AdminForum;
pub use log::AdminLog;
pub use reports::AdminReports;
pub use settings::AdminSettings;
pub use users::AdminUsers;

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::components::Outlet;
use leptos_router::hooks::use_location;

use crate::api::admin::fetch_admin_stats;
use crate::components::{ErrorBox, Icon, Spinner};
use crate::state::use_session;

#[component]
pub fn AdminLayout() -> impl IntoView {
    let session = use_session();
    let location = use_location();
    let link = move |href: &'static str, icon: &'static str, label: &'static str, admin_only: bool| {
        let active = move || {
            let p = location.pathname.get();
            if href == "/admin" { p == "/admin" || p == "/admin/" } else { p.starts_with(href) }
        };
        view! {
            <Show when=move || !admin_only || session.is_admin()>
                <a class="subnav-item" class:active=active href=href><Icon name=icon size=16/>{label}</a>
            </Show>
        }
    };
    view! {
        <Title text="Verwaltung"/>
        <Show when=move || session.is_mod() fallback=|| view! {
            <div class="empty-state big"><Icon name="shield" size=40/><p>"Nur für Moderation und Admins."</p></div>
        }>
            <div class="admin">
                <nav class="subnav">
                    {link("/admin", "activity", "Übersicht", false)}
                    {link("/admin/reports", "flag", "Meldungen", false)}
                    {link("/admin/users", "users", "Benutzer", false)}
                    {link("/admin/forum", "message-square", "Forum", true)}
                    {crate::pages::admin_ext::admin_nav()}
                    {link("/admin/settings", "sliders", "Einstellungen", true)}
                    {link("/admin/log", "list", "Protokoll", false)}
                </nav>
                <div class="admin-body">
                    <Outlet/>
                </div>
            </div>
        </Show>
    }
}

#[component]
pub fn AdminDashboard() -> impl IntoView {
    let stats = Resource::new(|| (), |_| fetch_admin_stats());
    view! {
        <h1>"Übersicht"</h1>
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match stats.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(s) => {
                        let card = |icon: &'static str, label: &'static str, value: String, href: Option<&'static str>| {
                            let inner = view! {
                                <Icon name=icon size=22/>
                                <span class="stat-value">{value}</span>
                                <span class="stat-label">{label}</span>
                            };
                            match href {
                                Some(h) => view! { <a class="stat-card card" href=h>{inner}</a> }.into_any(),
                                None => view! { <div class="stat-card card">{inner}</div> }.into_any(),
                            }
                        };
                        view! {
                            <div class="stat-grid">
                                {card("users", "Mitglieder", s.users.to_string(), Some("/admin/users"))}
                                {card("user-plus", "Neu (7 Tage)", s.users_new_7d.to_string(), None)}
                                {card("radio", "Online", s.online_now.to_string(), None)}
                                {card("flag", "Offene Meldungen", s.open_reports.to_string(), Some("/admin/reports"))}
                                {card("message-square", "Themen", s.topics.to_string(), None)}
                                {card("message-circle", "Beiträge", s.posts.to_string(), None)}
                                {card("hash", "Chatnachrichten", s.chat_messages.to_string(), None)}
                                {card("book-open", "Blogposts", s.blog_posts.to_string(), None)}
                                {card("image", "Bilder", s.images.to_string(), None)}
                                {card("file", "Dateien", s.files.to_string(), None)}
                                {card("hard-drive", "Speicher", shared::util::format_bytes(s.storage_bytes), None)}
                                {card("layout", "CMS-Einträge", s.cms_entries.to_string(), None)}
                            </div>
                        }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}
