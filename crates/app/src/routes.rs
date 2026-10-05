use leptos::prelude::*;
use leptos_router::components::{ParentRoute, Route, Routes};
use leptos_router::path;

use crate::pages::admin::{
    AdminDashboard, AdminForum, AdminLayout, AdminLog, AdminReports, AdminSettings, AdminUsers,
};
use crate::pages::chat::ChatPage;
use crate::pages::files::{FilesPage, SharePage};
use crate::pages::auth::{ForgotPasswordPage, LoginPage, RegisterPage, ResetPasswordPage};
use crate::pages::forum::{CategoryPage, ForumIndex, NewTopicPage, TagPage, TopicPage};
use crate::pages::home::HomePage;
use crate::pages::members::MembersPage;
use crate::pages::not_found::NotFound;
use crate::pages::notifications::NotificationsPage;
use crate::pages::profile::ProfilePage;
use crate::pages::search::SearchPage;
use crate::pages::settings::SettingsPage;

#[component]
pub fn AppRoutes() -> impl IntoView {
    view! {
        <Routes fallback=|| view! { <NotFound/> }>
            <Route path=path!("/") view=HomePage/>
            <Route path=path!("/login") view=LoginPage/>
            <Route path=path!("/register") view=RegisterPage/>
            <Route path=path!("/forgot-password") view=ForgotPasswordPage/>
            <Route path=path!("/reset-password") view=ResetPasswordPage/>
            <Route path=path!("/u/:username") view=ProfilePage/>
            <Route path=path!("/members") view=MembersPage/>
            <Route path=path!("/settings") view=SettingsPage/>
            <Route path=path!("/notifications") view=NotificationsPage/>
            <Route path=path!("/search") view=SearchPage/>

            <Route path=path!("/chat") view=ChatPage/>
            <Route path=path!("/chat/:id") view=ChatPage/>
            <Route path=path!("/files") view=FilesPage/>
            <Route path=path!("/s/:token") view=SharePage/>

            <Route path=path!("/forum") view=ForumIndex/>
            <Route path=path!("/forum/new") view=NewTopicPage/>
            <Route path=path!("/forum/c/:id") view=CategoryPage/>
            <Route path=path!("/forum/t/:id") view=TopicPage/>
            <Route path=path!("/forum/tag/:tag") view=TagPage/>

            <ParentRoute path=path!("/admin") view=AdminLayout>
                <Route path=path!("") view=AdminDashboard/>
                <Route path=path!("users") view=AdminUsers/>
                <Route path=path!("reports") view=AdminReports/>
                <Route path=path!("forum") view=AdminForum/>
                <Route path=path!("settings") view=AdminSettings/>
                <Route path=path!("log") view=AdminLog/>
            </ParentRoute>
        </Routes>
    }
}

/// Sidebar entries of optional modules (added phase by phase).
pub fn module_nav() -> impl IntoView {
    use crate::components::layout::NavItem;
    let session = crate::state::use_session();
    let m = move || session.site.with(|s| s.modules.clone());
    let chat_unread = use_context::<crate::pages::chat::ChatUnread>()
        .map(|c| c.0)
        .unwrap_or_else(|| RwSignal::new(0));
    view! {
        <Show when=move || m().chat && session.logged_in()>
            <NavItem href="/chat" icon="message-circle" label="Chat" badge=Signal::derive(move || chat_unread.get())/>
        </Show>
        <Show when=move || m().files && session.logged_in()>
            <NavItem href="/files" icon="hard-drive" label="Dateien"/>
        </Show>
    }
}

/// Extra entries in the user menu (e.g. "Mein Blog").
pub fn user_menu_items() -> impl IntoView {}
