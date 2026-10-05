#![recursion_limit = "256"]

pub mod api;
pub mod app;
pub mod components;
pub mod error;
pub mod pages;
pub mod realtime;
pub mod routes;
pub mod state;
pub mod upload;

#[cfg(feature = "ssr")]
pub mod ssr;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
