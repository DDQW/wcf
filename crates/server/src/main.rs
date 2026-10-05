use std::net::SocketAddr;

use app::app::{shell, RouteDefs};
use axum::extract::FromRef;
use axum::Router;
use backend::{AppState, Config};
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use sqlx::postgres::PgPoolOptions;
use tower_sessions::cookie::time::Duration as CookieDuration;
use tower_sessions::cookie::SameSite;
use tower_sessions::session_store::ExpiredDeletion;
use tower_sessions::{Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::PostgresStore;
use tracing_subscriber::EnvFilter;

/// Router state: Leptos options for rendering plus the backend state.
#[derive(Clone)]
struct ServerState {
    leptos: LeptosOptions,
    app: AppState,
}

impl FromRef<ServerState> for LeptosOptions {
    fn from_ref(s: &ServerState) -> Self {
        s.leptos.clone()
    }
}

impl FromRef<ServerState> for AppState {
    fn from_ref(s: &ServerState) -> Self {
        s.app.clone()
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_sessions=warn")),
        )
        .init();

    let conf = get_configuration(None).expect("failed to read leptos configuration");
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;

    let config = Config::from_env();
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await
        .expect("failed to connect to Postgres");
    backend::migrate(&pool).await.expect("failed to run migrations");
    if let Err(e) = backend::maintenance::rerender_if_needed(&pool).await {
        tracing::warn!(error = %e, "markdown re-render failed");
    }

    let session_store = PostgresStore::new(pool.clone());
    session_store
        .migrate()
        .await
        .expect("failed to migrate session store");
    let cleanup_store = session_store.clone();
    tokio::spawn(async move {
        let _ = cleanup_store
            .continuously_delete_expired(tokio::time::Duration::from_secs(3600))
            .await;
    });
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(config.cookie_secure)
        .with_same_site(SameSite::Lax)
        .with_http_only(true)
        .with_expiry(Expiry::OnInactivity(CookieDuration::days(30)));

    let app_state = AppState::new(config, pool)
        .await
        .expect("failed to initialise app state");
    backend::jobs::spawn(app_state.clone());
    backend::watch_media::start(app_state.clone()).await;
    backend::watch_live::start(app_state.clone());

    let state = ServerState {
        leptos: leptos_options.clone(),
        app: app_state.clone(),
    };
    let routes = generate_route_list(RouteDefs);
    let ctx_state = app_state.clone();
    let fallback_state = app_state.clone();

    let mut router: Router<ServerState> = Router::new()
        .leptos_routes_with_context(
            &state,
            routes,
            move || provide_context(ctx_state.clone()),
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .merge(backend::routes::<ServerState>())
        .fallback(leptos_axum::file_and_error_handler_with_context::<ServerState, _>(
            move || provide_context(fallback_state.clone()),
            shell,
        ));
    for layer in backend::http::security_headers() {
        router = router.layer(layer);
    }
    let app = router
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(session_layer)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("server error");
}
