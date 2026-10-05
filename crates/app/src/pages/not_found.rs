use leptos::prelude::*;
use leptos_meta::Title;

use crate::components::Icon;

#[component]
pub fn NotFound() -> impl IntoView {
    #[cfg(feature = "ssr")]
    crate::ssr::set_status(404);
    view! {
        <Title text="Nicht gefunden"/>
        <div class="empty-state big">
            <Icon name="alert" size=48/>
            <h1>"Seite nicht gefunden"</h1>
            <p class="muted">"Diese Seite gibt es nicht (mehr)."</p>
            <a class="btn btn-primary" href="/">"Zur Startseite"</a>
        </div>
    }
}

/// Inline "not found / no access" state for pages with a missing entity.
#[component]
pub fn Missing(#[prop(into)] text: String) -> impl IntoView {
    view! {
        <div class="empty-state big">
            <Icon name="alert" size=40/>
            <p>{text}</p>
            <a class="btn" href="/">"Zur Startseite"</a>
        </div>
    }
}
