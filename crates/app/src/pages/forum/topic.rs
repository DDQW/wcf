use chrono::{DateTime, Utc};
use leptos::html;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use shared::forum::{PostView, SubscriptionLevel, ThreadBlock, TopicDetail, TopicUpdate};
use shared::ws::{ServerMsg, Topic};
use uuid::Uuid;

use super::list::{CategoryChip, TagChip};
use super::thread::ThreadBlockView;
use crate::api::forum::{
    create_post, delete_topic, forum_categories, get_topic, list_blocks, move_topic, post_source,
    restore_topic, set_topic_locked, set_topic_pinned, set_topic_subscription, update_topic,
};
use crate::components::common::confirm;
use crate::components::{ErrorBox, Icon, InfiniteSentinel, MarkdownEditor, Modal, Spinner, TimeAgo};
use crate::error::AppError;
use crate::pages::not_found::Missing;
use crate::realtime::use_realtime;
use crate::state::{use_session, use_toasts};

/// Shared state of an open topic, provided as context to posts.
#[derive(Clone, Copy)]
pub struct TopicCtx {
    pub topic_id: Uuid,
    pub first_post_id: Option<Uuid>,
    pub detail: RwSignal<TopicDetail>,
    /// Read marker before this visit; newer posts are marked "neu".
    pub last_read: Option<DateTime<Utc>>,
    pub highlight: RwSignal<Option<Uuid>>,
    pub apply_post: Callback<PostView>,
    pub mark_deleted: Callback<Uuid>,
    /// (post, quote?) → prepare the composer.
    pub start_reply: Callback<(PostView, bool)>,
    pub on_topic_deleted: Callback<()>,
}

type Blocks = RwSignal<Vec<(Uuid, ArcRwSignal<ThreadBlock>)>>;

fn parse_uuid(s: Option<String>) -> Option<Uuid> {
    s.and_then(|s| Uuid::parse_str(&s).ok())
}

/// Smoothly scrolls a post into the middle of the viewport.
pub fn scroll_to_post(id: Uuid) {
    #[cfg(feature = "hydrate")]
    set_timeout(
        move || {
            if let Some(el) = document().get_element_by_id(&format!("post-{id}")) {
                let opts = web_sys::ScrollIntoViewOptions::new();
                opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                opts.set_block(web_sys::ScrollLogicalPosition::Center);
                el.scroll_into_view_with_scroll_into_view_options(&opts);
            }
        },
        std::time::Duration::from_millis(120),
    );
    #[cfg(not(feature = "hydrate"))]
    let _ = id;
}

#[component]
pub fn TopicPage() -> impl IntoView {
    let params = use_params_map();
    let id = move || parse_uuid(params.read().get("id"));
    let topic = Resource::new(id, |id| async move {
        match id {
            Some(id) => get_topic(id).await,
            None => Err(AppError::NotFound("Thema nicht gefunden.".into())),
        }
    });
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match topic.await {
                    Ok(t) => view! { <TopicView t=t/> }.into_any(),
                    Err(e) if e.is_not_found() => view! { <Missing text="Dieses Thema gibt es nicht (mehr) oder du hast keinen Zugriff."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

fn find_block(blocks: Blocks, root: Uuid) -> Option<ArcRwSignal<ThreadBlock>> {
    blocks.with_untracked(|v| v.iter().find(|(id, _)| *id == root).map(|(_, b)| b.clone()))
}

#[component]
fn TopicView(t: TopicDetail) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let rt = use_realtime();
    let navigate = use_navigate();
    let query = use_query_map();
    let topic_id = t.id;
    let category_id = t.category.id;
    let detail = RwSignal::new(t.clone());

    let initial_from = query.with_untracked(|q| {
        parse_uuid(q.get("post")).or_else(|| q.get("unread").and(t.first_unread_post))
    });
    let from_post = RwSignal::new(initial_from);
    let highlight = RwSignal::new(initial_from);
    let blocks: Blocks = RwSignal::new(Vec::new());
    let next = RwSignal::new(None::<String>);
    let prev = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let loading_prev = RwSignal::new(false);
    let new_posts = RwSignal::new(0usize);
    let scrolled = RwSignal::new(false);
    let first = Resource::new(move || from_post.get(), move |from| list_blocks(topic_id, None, None, from));

    // --- state updates (local actions and realtime events) ---------------
    let update_existing = move |p: PostView| -> bool {
        if p.parent_post_id.is_none() {
            if let Some(b) = find_block(blocks, p.id) {
                b.update(|b| b.root = p);
                return true;
            }
        } else if let Some(b) = find_block(blocks, p.thread_root_id) {
            let mut found = false;
            b.update(|b| {
                if let Some(existing) = b.replies.iter_mut().find(|r| r.id == p.id) {
                    *existing = p;
                    found = true;
                }
            });
            return found;
        }
        false
    };
    let apply_post = Callback::new(move |p: PostView| {
        if update_existing(p.clone()) {
            return;
        }
        if p.parent_post_id.is_none() {
            if next.get_untracked().is_none() {
                blocks.update(|v| {
                    v.push((p.id, ArcRwSignal::new(ThreadBlock { root: p, replies: Vec::new(), reply_total: 0 })))
                });
            } else {
                new_posts.update(|n| *n += 1);
            }
        } else if let Some(b) = find_block(blocks, p.thread_root_id) {
            b.update(|b| {
                b.replies.push(p);
                b.reply_total += 1;
            });
        }
    });
    let mark_deleted = Callback::new(move |post_id: Uuid| {
        let staff = session.is_mod();
        let clear = |p: &mut PostView| {
            p.is_deleted = true;
            if !staff {
                p.body_html.clear();
            }
        };
        for (_, b) in blocks.get_untracked() {
            b.update(|b| {
                if b.root.id == post_id {
                    clear(&mut b.root);
                }
                for r in b.replies.iter_mut().filter(|r| r.id == post_id) {
                    clear(r);
                }
            });
        }
    });
    let on_topic_deleted = Callback::new({
        let navigate = navigate.clone();
        move |_| {
            toasts.success("Thema gelöscht.");
            navigate(&format!("/forum/c/{category_id}"), Default::default());
        }
    });

    // --- composer -----------------------------------------------------------
    let body = RwSignal::new(String::new());
    let reply_to = RwSignal::new(None::<PostView>);
    let composer_ref = NodeRef::<html::Textarea>::new();
    let sending = RwSignal::new(false);
    let focus_composer = move || {
        #[cfg(feature = "hydrate")]
        if let Some(el) = composer_ref.get() {
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_behavior(web_sys::ScrollBehavior::Smooth);
            opts.set_block(web_sys::ScrollLogicalPosition::Center);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
            let _ = el.focus();
        }
    };
    let start_reply = Callback::new(move |(post, quote): (PostView, bool)| {
        if !session.logged_in() {
            toasts.info("Melde dich an, um zu antworten.", Some("/login".into()));
            return;
        }
        let username = post.author.username.clone();
        let id = post.id;
        reply_to.set(Some(post));
        if quote {
            leptos::task::spawn_local(async move {
                if let Ok(src) = post_source(id).await {
                    let quoted: Vec<String> = src.lines().map(|l| format!("> {l}")).collect();
                    body.update(|b| {
                        if !b.trim().is_empty() {
                            b.push_str("\n\n");
                        }
                        b.push_str(&format!("> **@{username}** schrieb:\n{}\n\n", quoted.join("\n")));
                    });
                }
                focus_composer();
            });
        } else {
            focus_composer();
        }
    });
    let submit = Callback::new(move |_| {
        if body.with_untracked(|b| b.trim().is_empty()) || sending.get_untracked() {
            return;
        }
        sending.set(true);
        let parent = reply_to.get_untracked().map(|p| p.id);
        let text = body.get_untracked();
        leptos::task::spawn_local(async move {
            match create_post(topic_id, parent, text).await {
                Ok(p) => {
                    body.set(String::new());
                    reply_to.set(None);
                    let id = p.id;
                    let is_root = p.parent_post_id.is_none();
                    if is_root && next.get_untracked().is_some() {
                        from_post.set(Some(id));
                    } else {
                        apply_post.run(p);
                    }
                    highlight.set(Some(id));
                    scroll_to_post(id);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            sending.set(false);
        });
    });

    provide_context(TopicCtx {
        topic_id,
        first_post_id: t.first_post_id,
        detail,
        last_read: t.last_read_at,
        highlight,
        apply_post,
        mark_deleted,
        start_reply,
        on_topic_deleted,
    });

    // --- realtime -----------------------------------------------------------
    rt.subscribe_scoped(Topic::ForumTopic(topic_id));
    rt.on_message(move |msg| match msg {
        ServerMsg::ForumPostCreated { post } if post.topic_id == topic_id => apply_post.run(post.clone()),
        ServerMsg::ForumPostUpdated { post } if post.topic_id == topic_id => {
            update_existing(post.clone());
        }
        ServerMsg::ForumPostDeleted { topic_id: tid, post_id } if *tid == topic_id => mark_deleted.run(*post_id),
        ServerMsg::ForumTopicUpdated { topic_id: tid } if *tid == topic_id => {
            leptos::task::spawn_local(async move {
                if let Ok(d) = get_topic(topic_id).await {
                    detail.set(d);
                }
            });
        }
        _ => {}
    });
    // After a reconnect, reload to pick up anything missed.
    Effect::new(move |prev_epoch: Option<u64>| {
        let epoch = rt.epoch.get();
        if prev_epoch.is_some_and(|p| p > 0 && p != epoch) {
            first.refetch();
        }
        epoch
    });

    // Permalink clicks within the page.
    Effect::new(move |prev_param: Option<Option<Uuid>>| {
        let p = query.with(|q| parse_uuid(q.get("post")));
        if let (Some(prev_param), Some(id)) = (prev_param, p) {
            if prev_param != Some(id) {
                highlight.set(Some(id));
                #[cfg(feature = "hydrate")]
                if document().get_element_by_id(&format!("post-{id}")).is_some() {
                    scroll_to_post(id);
                } else {
                    scrolled.set(false);
                    from_post.set(Some(id));
                }
            }
        }
        p
    });
    // Scroll to the target post once its block is rendered.
    Effect::new(move |_| {
        let has_blocks = blocks.with(|b| !b.is_empty());
        if has_blocks && !scrolled.get_untracked() {
            if let Some(id) = highlight.get_untracked() {
                scrolled.set(true);
                scroll_to_post(id);
            }
        }
    });

    let load_next = move |_| {
        let Some(c) = next.get_untracked() else { return };
        loading.set(true);
        leptos::task::spawn_local(async move {
            match list_blocks(topic_id, Some(c), None, None).await {
                Ok(page) => {
                    let known: Vec<Uuid> = blocks.with_untracked(|v| v.iter().map(|(id, _)| *id).collect());
                    blocks.update(|v| {
                        v.extend(
                            page.blocks
                                .into_iter()
                                .filter(|b| !known.contains(&b.root.id))
                                .map(|b| (b.root.id, ArcRwSignal::new(b))),
                        )
                    });
                    next.set(page.next_cursor);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            loading.set(false);
        });
    };
    let load_prev = move |_| {
        let Some(c) = prev.get_untracked() else { return };
        loading_prev.set(true);
        leptos::task::spawn_local(async move {
            match list_blocks(topic_id, None, Some(c), None).await {
                Ok(page) => {
                    blocks.update(|v| {
                        let mut new: Vec<(Uuid, ArcRwSignal<ThreadBlock>)> =
                            page.blocks.into_iter().map(|b| (b.root.id, ArcRwSignal::new(b))).collect();
                        new.append(v);
                        *v = new;
                    });
                    prev.set(page.prev_cursor);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            loading_prev.set(false);
        });
    };
    let show_new = move |_| {
        new_posts.set(0);
        load_next(());
    };
    let jump_unread = move |_| {
        if let Some(id) = detail.get_untracked().first_unread_post {
            highlight.set(Some(id));
            scrolled.set(false);
            from_post.set(Some(id));
        }
    };

    view! {
        <Title text=move || detail.with(|d| d.title.clone())/>
        <TopicHeader/>
        <Show when=move || detail.with(|d| d.first_unread_post.is_some()) && from_post.get().is_none()>
            <button class="btn btn-sm jump-unread" on:click=jump_unread><Icon name="arrow-down" size=14/>"Zum ersten ungelesenen Beitrag"</button>
        </Show>
        <Show when=move || prev.with(|p| p.is_some())>
            <button class="btn btn-ghost btn-block" disabled=move || loading_prev.get() on:click=load_prev>
                <Icon name="chevron-up" size=16/>"Frühere Beiträge laden"
            </button>
        </Show>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match first.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(page) => {
                        blocks.set(page.blocks.into_iter().map(|b| (b.root.id, ArcRwSignal::new(b))).collect());
                        next.set(page.next_cursor);
                        prev.set(page.prev_cursor);
                        view! {
                            <div class="thread-list">
                                <For each=move || blocks.get() key=|(id, _)| *id children=|(_, b)| view! { <ThreadBlockView block=b/> }/>
                            </div>
                        }.into_any()
                    }
                }
            })}
        </Transition>
        <InfiniteSentinel on_visible=load_next enabled=Signal::derive(move || next.with(|n| n.is_some())) loading=loading/>
        <Show when=move || { new_posts.get() > 0 }>
            <button class="new-posts-banner" on:click=show_new>
                <Icon name="arrow-down" size=16/>{move || format!("{} neue Beiträge anzeigen", new_posts.get())}
            </button>
        </Show>

        <section class="composer card" id="composer">
            {move || {
                let d = detail.get();
                if !session.logged_in() {
                    view! {
                        <p class="muted"><a href=format!("/login?next=/forum/t/{topic_id}")>"Melde dich an"</a>", um zu antworten."</p>
                    }.into_any()
                } else if !d.can_reply {
                    view! { <p class="muted"><Icon name="lock" size=16/>"Dieses Thema ist gesperrt."</p> }.into_any()
                } else {
                    ().into_any()
                }
            }}
            <Show when=move || session.logged_in() && detail.with(|d| d.can_reply)>
                <h3>"Antworten"</h3>
                {move || reply_to.get().map(|p| view! {
                    <div class="reply-target">
                        <Icon name="corner-down-right" size=14/>
                        <span>"Antwort an " <strong>{p.author.display_name.clone()}</strong></span>
                        <button class="link-btn" on:click=move |_| reply_to.set(None)>"(nur auf das Thema antworten)"</button>
                    </div>
                })}
                <MarkdownEditor value=body rows=5 on_submit=submit textarea_ref=composer_ref placeholder="Deine Antwort …"/>
                <div class="row end">
                    <button class="btn btn-primary" disabled=move || sending.get() on:click=move |_| submit.run(())>
                        <Icon name="send" size=16/>"Antworten"
                    </button>
                </div>
            </Show>
        </section>
    }
}

#[component]
fn TopicHeader() -> impl IntoView {
    let ctx = expect_context::<TopicCtx>();
    let session = use_session();
    let toasts = use_toasts();
    let detail = ctx.detail;
    let topic_id = ctx.topic_id;
    let editing = RwSignal::new(false);
    let title = RwSignal::new(String::new());
    let tags = RwSignal::new(String::new());
    let move_open = RwSignal::new(false);
    let move_target = RwSignal::new(String::new());
    let categories = Resource::new(move || move_open.get(), |open| async move {
        if open { forum_categories().await } else { Ok(Vec::new()) }
    });

    let start_edit = move |_| {
        let d = detail.get_untracked();
        title.set(d.title.clone());
        tags.set(d.tags.join(", "));
        editing.set(true);
    };
    let save_edit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = TopicUpdate {
            title: title.get_untracked(),
            tags: tags.get_untracked().split(',').map(|s| s.to_string()).collect(),
        };
        leptos::task::spawn_local(async move {
            match update_topic(topic_id, input.clone()).await {
                Ok(()) => {
                    detail.update(|d| {
                        d.title = input.title.trim().to_string();
                        d.tags = shared::forum::normalize_tags(&input.tags);
                    });
                    editing.set(false);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let set_sub = move |ev: leptos::ev::Event| {
        let level = SubscriptionLevel::parse(&event_target_value(&ev));
        leptos::task::spawn_local(async move {
            match set_topic_subscription(topic_id, level).await {
                Ok(()) => {
                    detail.update(|d| d.subscription = level);
                    toasts.success(match level {
                        SubscriptionLevel::Watching => "Du wirst über jeden neuen Beitrag benachrichtigt.",
                        SubscriptionLevel::Normal => "Benachrichtigungen nur bei Antworten an dich.",
                        SubscriptionLevel::Muted => "Thema stummgeschaltet.",
                    });
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let toggle_pin = move |_| {
        let pinned = !detail.get_untracked().is_pinned;
        leptos::task::spawn_local(async move {
            match set_topic_pinned(topic_id, pinned).await {
                Ok(()) => detail.update(|d| d.is_pinned = pinned),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let toggle_lock = move |_| {
        let locked = !detail.get_untracked().is_locked;
        leptos::task::spawn_local(async move {
            match set_topic_locked(topic_id, locked).await {
                Ok(()) => detail.update(|d| {
                    d.is_locked = locked;
                    d.can_reply = !d.is_deleted && (!locked || d.can_moderate);
                }),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let remove = move |_| {
        if !confirm("Dieses Thema löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_topic(topic_id).await {
                Ok(()) => ctx.on_topic_deleted.run(()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let restore = move |_| {
        leptos::task::spawn_local(async move {
            match restore_topic(topic_id).await {
                Ok(()) => detail.update(|d| d.is_deleted = false),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let do_move = move |_| {
        let Ok(target) = Uuid::parse_str(&move_target.get_untracked()) else { return };
        leptos::task::spawn_local(async move {
            match move_topic(topic_id, target).await {
                Ok(()) => {
                    if let Ok(d) = get_topic(topic_id).await {
                        detail.set(d);
                    }
                    move_open.set(false);
                    toasts.success("Thema verschoben.");
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    view! {
        <div class="breadcrumbs">
            <a href="/forum">"Forum"</a><Icon name="chevron-right" size=14/>
            {move || { let d = detail.get(); view! { <CategoryChip name=d.category.name.clone() color=d.category.color.clone() id=d.category.id/> } }}
        </div>
        <header class="topic-header">
            {move || if editing.get() {
                view! {
                    <form class="stack-sm grow" on:submit=save_edit>
                        <input type="text" class="title-input" bind:value=title maxlength="200" required/>
                        <input type="text" bind:value=tags placeholder="Tags, durch Komma getrennt"/>
                        <div class="row gap">
                            <button class="btn btn-sm btn-primary">"Speichern"</button>
                            <button type="button" class="btn btn-sm" on:click=move |_| editing.set(false)>"Abbrechen"</button>
                        </div>
                    </form>
                }.into_any()
            } else {
                let d = detail.get();
                view! {
                    <div class="grow">
                        <h1 class="topic-h1">
                            {d.is_pinned.then(|| view! { <span class="topic-flag" title="Angepinnt"><Icon name="pin" size=18/></span> })}
                            {d.is_locked.then(|| view! { <span class="topic-flag" title="Gesperrt"><Icon name="lock" size=18/></span> })}
                            {d.title.clone()}
                        </h1>
                        <div class="topic-meta">
                            {d.tags.iter().map(|t| view! { <TagChip tag=t.clone()/> }).collect_view()}
                            <span class="muted small">
                                <Icon name="message-circle" size=13/>{d.reply_count}" Antworten · "
                                <Icon name="eye" size=13/>{d.view_count}" Aufrufe · erstellt "<TimeAgo ts=d.created_at/>
                            </span>
                        </div>
                        {d.is_deleted.then(|| view! { <div class="alert alert-error small"><Icon name="trash" size=16/>"Dieses Thema ist gelöscht (nur für Moderation sichtbar)."</div> })}
                    </div>
                }.into_any()
            }}
            <div class="topic-tools">
                <Show when=move || session.logged_in()>
                    <label class="select-label" title="Benachrichtigungen für dieses Thema">
                        <Icon name="bell" size=15/>
                        <select class="select-sm" prop:value=move || detail.with(|d| d.subscription.as_str().to_string()) on:change=set_sub>
                            <option value="watching">"Beobachten"</option>
                            <option value="normal">"Normal"</option>
                            <option value="muted">"Stumm"</option>
                        </select>
                    </label>
                </Show>
                <Show when=move || detail.with(|d| d.can_edit) && !editing.get()>
                    <button class="btn btn-sm" on:click=start_edit><Icon name="edit" size=14/>"Bearbeiten"</button>
                </Show>
                <Show when=move || detail.with(|d| d.can_moderate)>
                    <button class="btn btn-sm" on:click=toggle_pin>
                        <Icon name="pin" size=14/>{move || if detail.with(|d| d.is_pinned) { "Lösen" } else { "Anpinnen" }}
                    </button>
                    <button class="btn btn-sm" on:click=toggle_lock>
                        {move || view! { <Icon name=if detail.with(|d| d.is_locked) { "unlock" } else { "lock" } size=14/> }}
                        {move || if detail.with(|d| d.is_locked) { "Entsperren" } else { "Sperren" }}
                    </button>
                    <button class="btn btn-sm" on:click=move |_| move_open.set(true)><Icon name="arrow-right" size=14/>"Verschieben"</button>
                    {move || if detail.with(|d| d.is_deleted) {
                        view! { <button class="btn btn-sm" on:click=restore><Icon name="rotate-cw" size=14/>"Wiederherstellen"</button> }.into_any()
                    } else {
                        view! { <button class="btn btn-sm btn-danger-soft" on:click=remove><Icon name="trash" size=14/>"Löschen"</button> }.into_any()
                    }}
                </Show>
            </div>
        </header>
        <Modal open=move_open title="Thema verschieben">
            <div class="stack">
                <Suspense fallback=|| view! { <Spinner/> }>
                    {move || Suspend::new(async move {
                        let cats = categories.await.unwrap_or_default();
                        view! {
                            <select bind:value=move_target>
                                <option value="">"Kategorie wählen …"</option>
                                {cats.into_iter().map(|c| view! { <option value=c.id.to_string()>{c.name.clone()}</option> }).collect_view()}
                            </select>
                        }
                    })}
                </Suspense>
                <div class="row end"><button class="btn btn-primary" on:click=do_move>"Verschieben"</button></div>
            </div>
        </Modal>
    }
}
