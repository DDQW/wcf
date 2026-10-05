use leptos::prelude::*;
use shared::UserSummary;

use crate::api::users::mention_suggest;
use crate::components::common::sleep_ms;
use crate::components::{Avatar, Icon};
use crate::state::use_session;

/// Search-as-you-type picker for one or more users.
#[component]
pub fn UserPicker(
    selected: RwSignal<Vec<UserSummary>>,
    #[prop(optional)] single: bool,
    #[prop(optional, into)] placeholder: String,
) -> impl IntoView {
    let session = use_session();
    let query = RwSignal::new(String::new());
    let results = RwSignal::new(Vec::<UserSummary>::new());
    Effect::new(move |_| {
        let q = query.get();
        if q.trim().is_empty() {
            results.set(Vec::new());
            return;
        }
        leptos::task::spawn_local(async move {
            sleep_ms(180).await;
            if query.get_untracked() != q {
                return;
            }
            if let Ok(list) = mention_suggest(q).await {
                let me = session.user_id();
                let chosen: Vec<_> = selected.get_untracked().iter().map(|u| u.id).collect();
                results.set(list.into_iter().filter(|u| Some(u.id) != me && !chosen.contains(&u.id)).collect());
            }
        });
    });
    let pick = move |u: UserSummary| {
        selected.update(|v| {
            if single {
                v.clear();
            }
            if !v.iter().any(|x| x.id == u.id) {
                v.push(u);
            }
        });
        query.set(String::new());
        results.set(Vec::new());
    };
    view! {
        <div class="user-picker">
            <div class="chips">
                <For each=move || selected.get() key=|u| u.id children=move |u| {
                    let id = u.id;
                    view! {
                        <span class="chip">
                            <Avatar user=u.clone() size=20/>{u.display_name.clone()}
                            <button type="button" aria-label="Entfernen" on:click=move |_| selected.update(|v| v.retain(|x| x.id != id))>
                                <Icon name="x" size=12/>
                            </button>
                        </span>
                    }
                }/>
            </div>
            <input type="text" placeholder=if placeholder.is_empty() { "Name eingeben …".to_string() } else { placeholder } bind:value=query/>
            <Show when=move || !results.with(|r| r.is_empty())>
                <ul class="picker-results">
                    <For each=move || results.get() key=|u| u.id children=move |u| {
                        let pick_u = u.clone();
                        view! {
                            <li on:mousedown=move |ev| { ev.prevent_default(); pick(pick_u.clone()); }>
                                <Avatar user=u.clone() size=24 presence=true/>
                                <span>{u.display_name.clone()}</span>
                                <span class="muted small">"@"{u.username.clone()}</span>
                            </li>
                        }
                    }/>
                </ul>
            </Show>
        </div>
    }
}
