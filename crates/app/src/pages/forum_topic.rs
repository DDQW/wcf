use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;
use shared::PostView;
use std::collections::HashSet;
use uuid::Uuid;

use crate::app::CurrentUser;
use crate::components::post_block::{group_into_blocks, ThreadBlockView, COLLAPSE_THRESHOLD};
use crate::server_fns::forum_fns::{get_topic, list_posts, CreatePost};

#[component]
pub fn ForumTopicPage() -> impl IntoView {
    let params = use_params_map();
    let topic_id = move || params.read().get("id").and_then(|s| Uuid::parse_str(&s).ok());

    let topic = Resource::new(topic_id, |id| async move {
        match id {
            Some(id) => Some(get_topic(id).await),
            None => None,
        }
    });

    let posts = RwSignal::new(Vec::<PostView>::new());
    let next_cursor = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let loaded_once = RwSignal::new(false);
    let collapsed = RwSignal::new(HashSet::<Uuid>::new());
    let seen_blocks = RwSignal::new(HashSet::<Uuid>::new());

    let load_more = move || {
        let Some(tid) = topic_id() else {
            return;
        };
        if loading.get_untracked() {
            return;
        }
        loading.set(true);
        let cursor = next_cursor.get_untracked();
        leptos::task::spawn_local(async move {
            if let Ok(page) = list_posts(tid, cursor).await {
                posts.update(|p| p.extend(page.items));
                next_cursor.set(page.next_cursor);
            }
            loading.set(false);
            loaded_once.set(true);
        });
    };

    Effect::new(move |_| {
        let _ = topic_id();
        posts.set(Vec::new());
        next_cursor.set(None);
        loaded_once.set(false);
        load_more();
    });

    let blocks = Memo::new(move |_| group_into_blocks(&posts.get()));

    // Auto-collapse a thread block the first time it crosses the reply
    // threshold; deliberately client-only and not persisted (see plan).
    Effect::new(move |_| {
        for block in blocks.get() {
            seen_blocks.update(|seen| {
                if seen.insert(block.thread_root_id)
                    && block.posts.len().saturating_sub(1) > COLLAPSE_THRESHOLD
                {
                    collapsed.update(|c| {
                        c.insert(block.thread_root_id);
                    });
                }
            });
        }
    });

    let reply_target = RwSignal::new(None::<Uuid>);
    let on_reply = Callback::new(move |post_id: Uuid| reply_target.set(Some(post_id)));

    let CurrentUser(user_resource) = expect_context::<CurrentUser>();

    view! {
        <Suspense fallback=move || view! { <p>"Lade Thema..."</p> }>
            {move || Suspend::new(async move {
                match topic.await {
                    Some(Ok(t)) => view! {
                        <A href=format!("/forum/category/{}", t.category_id)>"< Zurück"</A>
                        <h1>{t.title.clone()}</h1>
                    }.into_any(),
                    _ => view! { <p class="error">"Thema nicht gefunden."</p> }.into_any(),
                }
            })}
        </Suspense>

        <div class="post-list">
            <For each=move || blocks.get() key=|b| b.thread_root_id let(block)>
                <ThreadBlockView block=block collapsed=collapsed on_reply=on_reply/>
            </For>
        </div>

        <div class="sentinel">
            {move || {
                if loading.get() {
                    view! { <p>"Lade..."</p> }.into_any()
                } else if next_cursor.get().is_some() {
                    view! {
                        <button on:click=move |_| load_more()>"Weitere Beiträge laden"</button>
                    }.into_any()
                } else if loaded_once.get() {
                    view! { <p class="muted">"Keine weiteren Beiträge."</p> }.into_any()
                } else {
                    ().into_any()
                }
            }}
        </div>

        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                let logged_in = matches!(user_resource.await, Ok(Some(_)));
                logged_in.then(|| topic_id().map(|tid| view! {
                    <ReplyForm
                        topic_id=tid
                        reply_target=reply_target
                        on_created=move || {
                            reply_target.set(None);
                            posts.set(Vec::new());
                            next_cursor.set(None);
                            loaded_once.set(false);
                            load_more();
                        }
                    />
                }))
            })}
        </Suspense>
    }
}

#[component]
fn ReplyForm(
    topic_id: Uuid,
    reply_target: RwSignal<Option<Uuid>>,
    on_created: impl Fn() + 'static + Copy,
) -> impl IntoView {
    let action = ServerAction::<CreatePost>::new();

    Effect::new(move |_| {
        if let Some(Ok(_)) = action.value().get() {
            on_created();
        }
    });

    view! {
        <div class="reply-form">
            {move || reply_target.get().map(|_| view! {
                <p class="muted">
                    "Antwort auf einen Beitrag "
                    <button type="button" on:click=move |_| reply_target.set(None)>"(abbrechen)"</button>
                </p>
            })}
            <ActionForm action=action>
                <input type="hidden" name="topic_id" value=topic_id.to_string()/>
                <input type="hidden" name="parent_post_id" value=move || reply_target.get().map(|id| id.to_string()).unwrap_or_default()/>
                <textarea name="body_md" placeholder="Antwort (Markdown)" required></textarea>
                <button type="submit">"Antworten"</button>
            </ActionForm>
            {move || action.value().get().and_then(|r| r.err()).map(|e| view! {
                <p class="error">{e.to_string()}</p>
            })}
        </div>
    }
}
