use leptos::prelude::*;
use leptos_router::components::{ParentRoute, Route, Routes};
use leptos_router::path;
use leptos_router::SsrMode;

use crate::pages::admin::{
    AdminCms, AdminCmsType, AdminDashboard, AdminForum, AdminLayout, AdminLog, AdminMenu,
    AdminReports, AdminSettings, AdminUsers,
};
use crate::pages::blog::{BlogEditorPage, BlogPage, BlogPostPage, BlogTagPage, BlogsIndex, MyBlogPage};
use crate::pages::calls::{CallRoomPage, CallsPage, DirectCallPage};
use crate::pages::chat::ChatPage;
use crate::pages::cms::{CmsEditorPage, CmsEntryPage, CmsIndex, CmsListPage, CmsStaticPage};
use crate::pages::files::{FilesPage, SharePage};
use crate::pages::gallery::{AlbumPage, GalleryIndex, ImagePage};
use crate::pages::auth::{ForgotPasswordPage, LoginPage, RegisterPage, ResetPasswordPage};
use crate::pages::forum::{CategoryPage, ForumIndex, NewTopicPage, TagPage, TopicPage};
use crate::pages::home::HomePage;
use crate::pages::members::MembersPage;
use crate::pages::not_found::NotFound;
use crate::pages::notifications::NotificationsPage;
use crate::pages::profile::ProfilePage;
use crate::pages::search::SearchPage;
use crate::pages::settings::SettingsPage;
use crate::pages::watch::{WatchChatPopout, WatchIndex, WatchPlayerPopout, WatchRoomPage};
use crate::pages::watch_console::WatchConsolePage;
use crate::pages::watch_library::WatchLibraryPage;

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

            <Route path=path!("/watch") view=WatchIndex/>
            <Route path=path!("/watch/library") view=WatchLibraryPage/>
            <Route path=path!("/watch/:slug") view=WatchRoomPage/>
            <Route path=path!("/watch/:slug/console") view=WatchConsolePage/>
            <Route path=path!("/watch/:slug/player") view=WatchPlayerPopout/>
            <Route path=path!("/watch/:slug/chat") view=WatchChatPopout/>

            <Route path=path!("/calls") view=CallsPage/>
            <Route path=path!("/calls/direct/:id") view=DirectCallPage/>
            <Route path=path!("/calls/:id") view=CallRoomPage/>

            <Route path=path!("/gallery") view=GalleryIndex/>
            <Route path=path!("/gallery/album/:id") view=AlbumPage ssr=SsrMode::Async/>
            <Route path=path!("/gallery/image/:id") view=ImagePage ssr=SsrMode::Async/>

            <Route path=path!("/blogs") view=BlogsIndex/>
            <Route path=path!("/blogs/tag/:tag") view=BlogTagPage/>
            <Route path=path!("/blog/:username") view=BlogPage/>
            <Route path=path!("/blog/:username/:slug") view=BlogPostPage ssr=SsrMode::Async/>
            <Route path=path!("/me/blog") view=MyBlogPage/>
            <Route path=path!("/me/blog/new") view=BlogEditorPage/>
            <Route path=path!("/me/blog/edit/:id") view=BlogEditorPage/>
            <Route path=path!("/c") view=CmsIndex/>
            <Route path=path!("/c/:type") view=CmsListPage/>
            <Route path=path!("/c/:type/:slug") view=CmsEntryPage/>
            <Route path=path!("/p/:slug") view=CmsStaticPage/>
            <Route path=path!("/cms/:type/new") view=CmsEditorPage/>
            <Route path=path!("/cms/edit/:id") view=CmsEditorPage/>
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
                <Route path=path!("cms") view=AdminCms/>
                <Route path=path!("cms/type/new") view=AdminCmsType/>
                <Route path=path!("cms/type/:id") view=AdminCmsType/>
                <Route path=path!("menu") view=AdminMenu/>
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
        <Show when=move || m().blogs>
            <NavItem href="/blogs" icon="book-open" label="Blogs"/>
        </Show>
        <Show when=move || m().watch && session.logged_in()>
            <NavItem href="/watch" icon="tv" label="Kino"/>
        </Show>
        <Show when=move || m().calls && session.logged_in()>
            <NavItem href="/calls" icon="video" label="Anrufe"/>
        </Show>
        <Show when=move || m().gallery>
            <NavItem href="/gallery" icon="image" label="Galerie"/>
        </Show>
        <Show when=move || m().files && session.logged_in()>
            <NavItem href="/files" icon="hard-drive" label="Dateien"/>
        </Show>
        <Show when=move || m().cms>
            <NavItem href="/c" icon="layout" label="Inhalte" exact=true/>
        </Show>
    }
}

/// Extra entries in the user menu (e.g. "Mein Blog").
pub fn user_menu_items() -> impl IntoView {
    use crate::components::Icon;
    let session = crate::state::use_session();
    view! {
        <Show when=move || session.site.with(|s| s.modules.blogs)>
            <a class="menu-item" href="/me/blog"><Icon name="book-open" size=16/>"Mein Blog"</a>
        </Show>
        <Show when=move || session.site.with(|s| s.modules.gallery)>
            <a class="menu-item" href="/gallery?tab=mine"><Icon name="image" size=16/>"Meine Alben"</a>
        </Show>
        <Show when=move || session.site.with(|s| s.modules.files)>
            <a class="menu-item" href="/files"><Icon name="hard-drive" size=16/>"Meine Dateien"</a>
        </Show>
    }
}
