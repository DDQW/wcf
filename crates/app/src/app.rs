use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Html, Meta, MetaTags, Stylesheet, Title};
use leptos_router::components::Router;
use shared::Theme;

use crate::components::layout::Shell;
use crate::components::Spinner;
use crate::realtime::Realtime;
use crate::state::{Clock, Session, Toasts};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="de">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <link rel="icon" href="/favicon.svg" type="image/svg+xml"/>
                // Apply the stored theme before first paint (no flash).
                <script>
                    "try{var t=localStorage.getItem('theme');if(t&&t!=='system')document.documentElement.dataset.theme=t;}catch(e){}"
                </script>
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

pub fn store_local_theme(theme: Theme) {
    #[cfg(feature = "hydrate")]
    if let Ok(Some(storage)) = window().local_storage() {
        let _ = storage.set_item("theme", theme.as_str());
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = theme;
}

fn load_local_theme() -> Option<Theme> {
    #[cfg(feature = "hydrate")]
    {
        window()
            .local_storage()
            .ok()
            .flatten()
            .and_then(|s| s.get_item("theme").ok().flatten())
            .map(|t| Theme::parse(&t))
    }
    #[cfg(not(feature = "hydrate"))]
    {
        None
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let session = Session::new();
    provide_context(session);
    provide_context(Toasts::new());
    let clock = Clock::new();
    provide_context(clock);
    let rt = Realtime::new(session);
    provide_context(rt);
    let theme = RwSignal::new(Theme::System);
    provide_context(theme);

    let bootstrap = Resource::new(|| (), |_| crate::api::auth::load_bootstrap());

    // Client-only wiring: realtime connection, clock, theme sync.
    Effect::new(move |_| {
        rt.start();
        #[cfg(feature = "hydrate")]
        {
            let _ = set_interval_with_handle(
                move || clock.0.set(crate::state::now_ms()),
                std::time::Duration::from_secs(30),
            );
        }
    });
    Effect::new(move |prev: Option<Option<uuid::Uuid>>| {
        let current = session.user_id();
        if let Some(prev) = prev {
            if prev != current {
                rt.reconnect();
            }
        }
        current
    });
    Effect::new(move |_| {
        let t = session
            .user
            .with(|u| u.as_ref().map(|u| u.theme))
            .or_else(load_local_theme)
            .unwrap_or(Theme::System);
        theme.set(t);
        store_local_theme(t);
    });

    let site_name = move || session.site.with(|s| s.site_name.clone());

    view! {
        <Stylesheet id="leptos" href="/pkg/woelkicommunityframework.css"/>
        <Html {..} data-theme=move || match theme.get() {
            Theme::System => None,
            t => Some(t.as_str()),
        }/>
        <Title formatter=move |text: String| {
            let site = site_name();
            if text.is_empty() { site } else { format!("{text} · {site}") }
        }/>
        <Meta name="description" content=move || session.site.with(|s| s.site_description.clone())/>
        <Router>
            <Suspense fallback=|| view! { <div class="boot"><Spinner label="Lade …"/></div> }>
                {move || Suspend::new(async move {
                    match bootstrap.await {
                        Ok(b) => {
                            session.apply(b);
                            view! { <Shell/> }.into_any()
                        }
                        Err(e) => view! {
                            <div class="boot">
                                <div class="alert alert-error">{format!("Die Seite konnte nicht geladen werden: {e}")}</div>
                            </div>
                        }.into_any(),
                    }
                })}
            </Suspense>
        </Router>
    }
}

/// The bare route table. The server uses it to register its handlers: in
/// `App` the routes sit behind the async bootstrap, where route discovery
/// (which does not await) cannot see them.
#[component]
pub fn RouteDefs() -> impl IntoView {
    view! {
        <Router>
            <crate::routes::AppRoutes/>
        </Router>
    }
}
