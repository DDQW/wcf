//! Profile actions contributed by optional modules (message, call …).

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use uuid::Uuid;

use crate::components::Icon;
use crate::state::{use_session, use_toasts};

pub fn profile_actions(user_id: Uuid) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let message = move |_| {
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match crate::api::chat::open_dm(user_id).await {
                Ok(id) => navigate(&format!("/chat/{id}"), Default::default()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Show when=move || session.site.with(|s| s.modules.chat)>
            <button class="btn btn-primary" on:click=message.clone()><Icon name="message-circle" size=16/>"Nachricht"</button>
        </Show>
        <crate::pages::calls::CallButtons user_id=user_id/>
    }
}
