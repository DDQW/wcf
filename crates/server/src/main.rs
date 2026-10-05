mod ws;

use app::app::{shell, App};
use axum::routing::get;
use axum::Router;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use sqlx::postgres::PgPoolOptions;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::PostgresStore;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    let conf = get_configuration(None).expect("failed to read leptos configuration");
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("failed to connect to Postgres");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");

    let session_store = PostgresStore::new(pool.clone());
    session_store
        .migrate()
        .await
        .expect("failed to migrate session store");
    let session_layer = SessionManagerLayer::new(session_store).with_secure(false);

    let routes = generate_route_list(App);

    let pool_for_context = pool.clone();
    let leptos_options_for_shell = leptos_options.clone();

    let app = Router::new()
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            move || provide_context(pool_for_context.clone()),
            move || shell(leptos_options_for_shell.clone()),
        )
        .route("/ws", get(ws::ws_handler))
        .fallback(leptos_axum::file_and_error_handler(shell))
        .layer(session_layer)
        .with_state(leptos_options);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app.into_make_service())
        .await
        .expect("server error");
}
