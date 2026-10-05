use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes, A},
    path,
};

use crate::pages::{
    forum_category::ForumCategoryPage, forum_topic::ForumTopicPage, home::HomePage,
    login::LoginPage, register::RegisterPage,
};
use crate::server_fns::auth_fns::{get_current_user, logout};
use shared::AuthUser;

/// Current logged-in user, re-fetched after login/register/logout so every
/// page reacts without a full reload.
#[derive(Copy, Clone)]
pub struct CurrentUser(pub Resource<Result<Option<AuthUser>, ServerFnError>>);

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="de">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let user_resource = Resource::new(|| (), |_| get_current_user());
    provide_context(CurrentUser(user_resource));

    view! {
        <Stylesheet id="leptos" href="/pkg/woelkicommunityframework.css"/>
        <Title text="Woelki Community"/>
        <Router>
            <header class="site-header">
                <A href="/">"Woelki Community"</A>
                <nav>
                    <Suspense fallback=|| ()>
                        {move || Suspend::new(async move {
                            match user_resource.await {
                                Ok(Some(user)) => view! {
                                    <span class="nav-user">{format!("Hallo, {}", user.display_name)}</span>
                                    <button on:click=move |_| {
                                        leptos::task::spawn_local(async move {
                                            let _ = logout().await;
                                            user_resource.refetch();
                                        });
                                    }>"Abmelden"</button>
                                }.into_any(),
                                _ => view! {
                                    <A href="/login">"Anmelden"</A>
                                    <A href="/register">"Registrieren"</A>
                                }.into_any(),
                            }
                        })}
                    </Suspense>
                </nav>
            </header>
            <main>
                <Routes fallback=|| view! { <p>"Seite nicht gefunden."</p> }>
                    <Route path=path!("/") view=HomePage/>
                    <Route path=path!("/login") view=LoginPage/>
                    <Route path=path!("/register") view=RegisterPage/>
                    <Route path=path!("/forum/category/:id") view=ForumCategoryPage/>
                    <Route path=path!("/forum/topic/:id") view=ForumTopicPage/>
                </Routes>
            </main>
        </Router>
    }
}
