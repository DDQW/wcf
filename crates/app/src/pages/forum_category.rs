use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;
use shared::TopicSummary;
use uuid::Uuid;

use crate::app::CurrentUser;
use crate::server_fns::forum_fns::{list_topics, CreateTopic};

#[component]
pub fn ForumCategoryPage() -> impl IntoView {
    let params = use_params_map();
    let category_id =
        move || params.read().get("id").and_then(|s| Uuid::parse_str(&s).ok());

    let topics = RwSignal::new(Vec::<TopicSummary>::new());
    let next_cursor = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let loaded_once = RwSignal::new(false);

    let load_more = move || {
        let Some(cid) = category_id() else {
            return;
        };
        if loading.get_untracked() {
            return;
        }
        loading.set(true);
        let cursor = next_cursor.get_untracked();
        leptos::task::spawn_local(async move {
            if let Ok(page) = list_topics(cid, cursor).await {
                topics.update(|t| t.extend(page.items));
                next_cursor.set(page.next_cursor);
            }
            loading.set(false);
            loaded_once.set(true);
        });
    };

    Effect::new(move |_| {
        let _ = category_id();
        topics.set(Vec::new());
        next_cursor.set(None);
        loaded_once.set(false);
        load_more();
    });

    let CurrentUser(user_resource) = expect_context::<CurrentUser>();

    view! {
        <A href="/">"< Kategorien"</A>
        <h1>"Themen"</h1>

        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                let logged_in = matches!(user_resource.await, Ok(Some(_)));
                logged_in.then(|| category_id().map(|cid| view! {
                    <CreateTopicForm category_id=cid on_created=move || {
                        topics.set(Vec::new());
                        next_cursor.set(None);
                        loaded_once.set(false);
                        load_more();
                    }/>
                }))
            })}
        </Suspense>

        <ul class="topic-list">
            <For each=move || topics.get() key=|t| t.id let(topic)>
                <li>
                    <A href=format!("/forum/topic/{}", topic.id)>{topic.title.clone()}</A>
                    <span class="muted">
                        {format!(" von {} · {} Antworten", topic.author_display_name, topic.reply_count)}
                    </span>
                </li>
            </For>
        </ul>

        <div class="sentinel">
            {move || {
                if loading.get() {
                    view! { <p>"Lade..."</p> }.into_any()
                } else if next_cursor.get().is_some() {
                    view! {
                        <button on:click=move |_| load_more()>"Weitere Themen laden"</button>
                    }.into_any()
                } else if loaded_once.get() {
                    view! { <p class="muted">"Keine weiteren Themen."</p> }.into_any()
                } else {
                    ().into_any()
                }
            }}
        </div>
    }
}

#[component]
fn CreateTopicForm(category_id: Uuid, on_created: impl Fn() + 'static + Copy) -> impl IntoView {
    let action = ServerAction::<CreateTopic>::new();

    Effect::new(move |_| {
        if let Some(Ok(_)) = action.value().get() {
            on_created();
        }
    });

    view! {
        <details class="new-topic">
            <summary>"Neues Thema erstellen"</summary>
            <ActionForm action=action>
                <input type="hidden" name="category_id" value=category_id.to_string()/>
                <input type="text" name="title" placeholder="Titel" required/>
                <textarea name="body_md" placeholder="Text (Markdown)" required></textarea>
                <button type="submit">"Erstellen"</button>
            </ActionForm>
            {move || action.value().get().and_then(|r| r.err()).map(|e| view! {
                <p class="error">{e.to_string()}</p>
            })}
        </details>
    }
}
