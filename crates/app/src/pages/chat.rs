use std::collections::HashMap;

use chrono::{DateTime, Utc};
use leptos::html;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use shared::chat::{
    ChannelDetail, ChannelKind, ChannelView, ChatMessageView, ChatRole, MessageKind, NewChannel,
};
use shared::files::{FileKind, FileView};
use shared::ws::{ClientMsg, ServerMsg, Topic};
use shared::{SubjectType, UserSummary};
use uuid::Uuid;

use crate::api::chat::*;
use crate::components::common::{confirm, copy_to_clipboard, local_date, local_time, origin, sleep_ms};
use crate::components::uploads::FileThumb;
use crate::components::user_picker::UserPicker;
use crate::components::{
    Avatar, EmptyState, ErrorBox, Icon, Modal, ReactionBar, ReportButton, RequireLogin, Spinner,
    TimeAgo,
};
use crate::realtime::use_realtime;
use crate::state::{use_session, use_toasts};

// ---------------------------------------------------------------------------
// Global unread badge
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct ChatUnread(pub RwSignal<i64>);

/// Provides the chat unread counter for the sidebar (called by the shell).
pub fn provide_chat_badge() {
    let session = use_session();
    let rt = use_realtime();
    let unread = RwSignal::new(0i64);
    provide_context(ChatUnread(unread));
    let refresh = move || {
        leptos::task::spawn_local(async move {
            if let Ok(n) = chat_unread_total().await {
                unread.set(n);
            }
        });
    };
    Effect::new(move |_| {
        rt.epoch.track();
        if session.logged_in() && session.site.with(|s| s.modules.chat) {
            refresh();
        } else {
            unread.set(0);
        }
    });
    rt.on_message(move |msg| {
        if let ServerMsg::ChatActivity { .. } = msg {
            refresh();
        }
    });
}

pub fn refresh_chat_badge() {
    if let Some(ChatUnread(u)) = use_context::<ChatUnread>() {
        leptos::task::spawn_local(async move {
            if let Ok(n) = chat_unread_total().await {
                u.set(n);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

#[component]
pub fn ChatPage() -> impl IntoView {
    view! {
        <Title text="Chat"/>
        <RequireLogin>
            <ChatLayout/>
        </RequireLogin>
    }
}

#[derive(Clone, Copy)]
struct ChatCtx {
    channels: RwSignal<Vec<ChannelView>>,
    reload_channels: Callback<()>,
    current: Memo<Option<Uuid>>,
}

#[component]
fn ChatLayout() -> impl IntoView {
    let rt = use_realtime();
    let params = use_params_map();
    let current = Memo::new(move |_| params.with(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok())));
    let channels = RwSignal::new(Vec::<ChannelView>::new());
    let reload_channels = Callback::new(move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(list) = my_channels().await {
                channels.set(list);
            }
        });
    });
    Effect::new(move |_| {
        rt.epoch.track();
        reload_channels.run(());
    });
    rt.on_message(move |msg| match msg {
        ServerMsg::ChatActivity { channel_id, mention } => {
            if current.get_untracked() == Some(*channel_id) {
                return;
            }
            let known = channels.with_untracked(|c| c.iter().any(|x| x.id == *channel_id));
            if known {
                channels.update(|list| {
                    if let Some(c) = list.iter_mut().find(|c| c.id == *channel_id) {
                        c.unread += 1;
                        if *mention {
                            c.mentions += 1;
                        }
                        c.last_message_at = Some(Utc::now());
                    }
                });
            } else {
                reload_channels.run(());
            }
        }
        ServerMsg::ChatChannelUpdated { .. } => reload_channels.run(()),
        _ => {}
    });
    // Opening a channel clears its badge locally.
    Effect::new(move |_| {
        if let Some(id) = current.get() {
            channels.update(|list| {
                if let Some(c) = list.iter_mut().find(|c| c.id == id) {
                    c.unread = 0;
                    c.mentions = 0;
                }
            });
        }
    });
    provide_context(ChatCtx {
        channels,
        reload_channels,
        current,
    });

    view! {
        <div class="chat" class:has-channel=move || current.get().is_some()>
            <ChannelSidebar/>
            {move || match current.get() {
                Some(id) => view! { <ChannelPane id=id/> }.into_any(),
                None => view! {
                    <div class="chat-empty">
                        <EmptyState icon="message-square" title="Wähle links einen Kanal oder starte eine Unterhaltung."/>
                    </div>
                }.into_any(),
            }}
        </div>
    }
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

#[component]
fn ChannelSidebar() -> impl IntoView {
    let ctx = expect_context::<ChatCtx>();
    let rt = use_realtime();
    let session = use_session();
    let new_channel = RwSignal::new(false);
    let new_dm = RwSignal::new(false);
    let browse = RwSignal::new(false);
    let can_create = move || session.is_mod() || session.site.with(|s| s.members_create_channels);

    let item = move |c: ChannelView| {
        let id = c.id;
        let active = move || ctx.current.get() == Some(id);
        let dm_user = c.dm_user.clone();
        view! {
            <a class="channel-item" class:active=active class:unread={c.unread > 0} class:muted-ch=c.muted href=c.url()>
                {match dm_user {
                    Some(u) => {
                        let uid = u.id;
                        view! {
                            <span class="ch-avatar"><Avatar user=u size=22/>
                                <span class="presence-dot" class:online=move || rt.is_online(uid)></span>
                            </span>
                        }.into_any()
                    }
                    None => view! { <Icon name=c.kind.icon() size=16/> }.into_any(),
                }}
                <span class="channel-name">{match c.kind {
                    ChannelKind::Public | ChannelKind::Private => c.name.clone(),
                    _ => c.name.clone(),
                }}</span>
                {(c.mentions > 0).then(|| view! { <span class="ch-badge mention">"@"{c.mentions}</span> })}
                {(c.unread > 0 && c.mentions == 0 && !c.muted).then(|| view! { <span class="ch-badge">{c.unread}</span> })}
            </a>
        }
    };

    let is_room = |c: &ChannelView| matches!(c.kind, ChannelKind::Public | ChannelKind::Private);
    let rooms = move || ctx.channels.get().into_iter().filter(|c| is_room(c) && !c.is_archived).collect::<Vec<_>>();
    let dms = move || ctx.channels.get().into_iter().filter(|c| c.kind.is_dm()).collect::<Vec<_>>();
    let archived = move || ctx.channels.get().into_iter().filter(|c| c.is_archived).collect::<Vec<_>>();
    let no_rooms = move || ctx.channels.with(|c| !c.iter().any(is_room));
    view! {
        <aside class="chat-sidebar">
            <div class="chat-sidebar-head">
                <h2>"Chat"</h2>
                <button class="icon-btn" title="Neue Nachricht" on:click=move |_| new_dm.set(true)><Icon name="edit"/></button>
            </div>
            <div class="chat-section">
                <div class="chat-section-head">
                    <span>"Kanäle"</span>
                    <button class="icon-btn small-btn" title="Kanäle durchsuchen" on:click=move |_| browse.set(true)><Icon name="search" size=15/></button>
                    <Show when=can_create>
                        <button class="icon-btn small-btn" title="Kanal erstellen" on:click=move |_| new_channel.set(true)><Icon name="plus" size=15/></button>
                    </Show>
                </div>
                <For
                    each=rooms
                    key=|c| (c.id, c.unread, c.mentions, c.name.clone(), c.muted)
                    children=item
                />
                <Show when=no_rooms>
                    <button class="link-btn small pad-x" on:click=move |_| browse.set(true)>"Kanälen beitreten …"</button>
                </Show>
            </div>
            <div class="chat-section">
                <div class="chat-section-head">
                    <span>"Direktnachrichten"</span>
                    <button class="icon-btn small-btn" title="Neue Nachricht" on:click=move |_| new_dm.set(true)><Icon name="plus" size=15/></button>
                </div>
                <For
                    each=dms
                    key=|c| (c.id, c.unread, c.mentions, c.name.clone(), c.muted)
                    children=item
                />
            </div>
            <Show when=move || ctx.channels.with(|c| c.iter().any(|c| c.is_archived))>
                <div class="chat-section">
                    <div class="chat-section-head"><span>"Archiviert"</span></div>
                    <For
                        each=archived
                        key=|c| c.id
                        children=item
                    />
                </div>
            </Show>
        </aside>
        <NewChannelModal open=new_channel/>
        <NewDmModal open=new_dm/>
        <BrowseModal open=browse/>
    }
}

#[component]
fn NewChannelModal(open: RwSignal<bool>) -> impl IntoView {
    let ctx = expect_context::<ChatCtx>();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let name = RwSignal::new(String::new());
    let topic = RwSignal::new(String::new());
    let private = RwSignal::new(false);
    let members = RwSignal::new(Vec::<UserSummary>::new());
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = NewChannel {
            name: name.get_untracked(),
            topic: topic.get_untracked(),
            private: private.get_untracked(),
            members: members.get_untracked().iter().map(|u| u.id).collect(),
        };
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match create_channel(input).await {
                Ok(id) => {
                    open.set(false);
                    name.set(String::new());
                    topic.set(String::new());
                    members.set(Vec::new());
                    ctx.reload_channels.run(());
                    navigate(&format!("/chat/{id}"), Default::default());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Modal open=open title="Kanal erstellen">
            <form class="stack" on:submit=submit.clone()>
                <label class="field"><span>"Name"</span><input type="text" required placeholder="z. B. spiele" bind:value=name/>
                    <small class="muted">{move || format!("#{}", shared::chat::normalize_channel_name(&name.get()))}</small></label>
                <label class="field"><span>"Thema (optional)"</span><input type="text" bind:value=topic/></label>
                <label class="check"><input type="checkbox" bind:checked=private/>"Privat – nur eingeladene Mitglieder"</label>
                <Show when=move || private.get()>
                    <div class="field"><span>"Mitglieder einladen"</span><UserPicker selected=members/></div>
                </Show>
                <div class="row end"><button class="btn btn-primary">"Erstellen"</button></div>
            </form>
        </Modal>
    }
}

#[component]
fn NewDmModal(open: RwSignal<bool>) -> impl IntoView {
    let ctx = expect_context::<ChatCtx>();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let members = RwSignal::new(Vec::<UserSummary>::new());
    let name = RwSignal::new(String::new());
    let start = move |_| {
        let ids: Vec<Uuid> = members.get_untracked().iter().map(|u| u.id).collect();
        if ids.is_empty() {
            return;
        }
        let navigate = navigate.clone();
        let group_name = name.get_untracked();
        leptos::task::spawn_local(async move {
            let res = if ids.len() == 1 { open_dm(ids[0]).await } else { create_group(ids, group_name).await };
            match res {
                Ok(id) => {
                    open.set(false);
                    members.set(Vec::new());
                    ctx.reload_channels.run(());
                    navigate(&format!("/chat/{id}"), Default::default());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Modal open=open title="Neue Nachricht">
            <div class="stack">
                <div class="field"><span>"An"</span><UserPicker selected=members placeholder="Personen suchen …"/></div>
                <Show when=move || { members.with(|m| m.len()) > 1 }>
                    <label class="field"><span>"Gruppenname (optional)"</span><input type="text" bind:value=name/></label>
                </Show>
                <div class="row end"><button class="btn btn-primary" on:click=start.clone()>"Unterhaltung starten"</button></div>
            </div>
        </Modal>
    }
}

#[component]
fn BrowseModal(open: RwSignal<bool>) -> impl IntoView {
    let ctx = expect_context::<ChatCtx>();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let list = Resource::new(move || open.get(), |o| async move { if o { browse_channels().await } else { Ok(Vec::new()) } });
    view! {
        <Modal open=open title="Kanäle">
            <Suspense fallback=|| view! { <Spinner/> }>
                {move || {
                    Suspend::new(async move {
                        let channels = list.await.unwrap_or_default();
                        if channels.is_empty() {
                            return view! { <p class="muted">"Noch keine öffentlichen Kanäle."</p> }.into_any();
                        }
                        view! {
                            <ul class="browse-list">
                                {channels.into_iter().map(|c| {
                                    let id = c.id;
                                    let joined = c.joined;
                                    view! {
                                        <li>
                                            <div class="grow">
                                                <strong>"#"{c.name.clone()}</strong>
                                                <div class="muted small">{c.topic.clone()} " · " {c.member_count} " Mitglieder"</div>
                                            </div>
                                            <button class="btn btn-sm" class:btn-primary={!joined} on:click=move |_| {
                                                leptos::task::spawn_local(async move {
                                                    if !joined {
                                                        if let Err(e) = join_channel(id).await { toasts.error(e.to_string()); return; }
                                                    }
                                                    open.set(false);
                                                    ctx.reload_channels.run(());
                                                    go.run(format!("/chat/{id}"));
                                                });
                                            }>{if joined { "Öffnen" } else { "Beitreten" }}</button>
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    })
                }}
            </Suspense>
        </Modal>
    }
}

// ---------------------------------------------------------------------------
// Channel pane
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct PaneCtx {
    channel_id: Uuid,
    detail: RwSignal<Option<ChannelDetail>>,
    reply_to: RwSignal<Option<ChatMessageView>>,
    editing: RwSignal<Option<Uuid>>,
    apply: Callback<ChatMessageView>,
    highlight: RwSignal<Option<Uuid>>,
}

fn scroll_to_bottom(el: &NodeRef<html::Div>) {
    if let Some(div) = el.get_untracked() {
        div.set_scroll_top(div.scroll_height());
    }
}

fn near_bottom(el: &NodeRef<html::Div>) -> bool {
    el.get_untracked()
        .map(|d| d.scroll_height() - d.scroll_top() - d.client_height() < 120)
        .unwrap_or(true)
}

#[component]
fn ChannelPane(id: Uuid) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let rt = use_realtime();
    let query = use_query_map();
    let navigate = use_navigate();
    let chat = expect_context::<ChatCtx>();
    let detail = RwSignal::new(None::<ChannelDetail>);
    let detail_error = RwSignal::new(None::<String>);
    let messages = RwSignal::new(Vec::<ChatMessageView>::new());
    let older = RwSignal::new(None::<String>);
    let newer = RwSignal::new(None::<String>);
    let loading_older = RwSignal::new(false);
    let loaded = RwSignal::new(false);
    let new_below = RwSignal::new(0usize);
    let typing = RwSignal::new(HashMap::<Uuid, (UserSummary, f64)>::new());
    let reply_to = RwSignal::new(None::<ChatMessageView>);
    let editing = RwSignal::new(None::<Uuid>);
    let highlight = RwSignal::new(query.with_untracked(|q| q.get("m").and_then(|m| Uuid::parse_str(&m).ok())));
    let panel = RwSignal::new(None::<&'static str>);
    let settings_open = RwSignal::new(false);
    let scroller = NodeRef::<html::Div>::new();
    let last_read = RwSignal::new(None::<DateTime<Utc>>);

    let load_detail = move || {
        leptos::task::spawn_local(async move {
            match get_channel_detail(id).await {
                Ok(d) => {
                    if last_read.get_untracked().is_none() {
                        last_read.set(d.last_read_at);
                    }
                    detail.set(Some(d));
                }
                Err(e) => detail_error.set(Some(e.to_string())),
            }
        });
    };
    let mark_read = move || {
        leptos::task::spawn_local(async move {
            let _ = mark_channel_read(id).await;
            refresh_chat_badge_with(chat_unread_signal());
        });
    };
    let load_initial = move |around: Option<Uuid>| {
        loaded.set(false);
        leptos::task::spawn_local(async move {
            match list_messages(id, None, None, around).await {
                Ok(page) => {
                    messages.set(page.messages);
                    older.set(page.older_cursor);
                    newer.set(page.newer_cursor);
                    loaded.set(true);
                    sleep_ms(30).await;
                    match around {
                        Some(m) => scroll_to_message(m),
                        None => scroll_to_bottom(&scroller),
                    }
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    Effect::new(move |_| {
        load_detail();
        load_initial(highlight.get_untracked());
        mark_read();
    });
    rt.subscribe_scoped(Topic::Chat(id));
    // Reconnect → reload so nothing gets lost.
    Effect::new(move |prev: Option<u64>| {
        let e = rt.epoch.get();
        if prev.is_some_and(|p| p != e) {
            load_initial(None);
        }
        e
    });

    let apply = Callback::new(move |m: ChatMessageView| {
        messages.update(|list| {
            if let Some(existing) = list.iter_mut().find(|x| x.id == m.id) {
                let mine: Vec<String> = existing.reactions.iter().filter(|r| r.me).map(|r| r.emoji.clone()).collect();
                let mut m = m;
                for r in m.reactions.iter_mut() {
                    r.me = mine.contains(&r.emoji);
                }
                *existing = m;
            } else {
                list.push(m);
            }
        });
    });

    rt.on_message(move |msg| match msg {
        ServerMsg::ChatMessage { message } if message.channel_id == id => {
            if newer.get_untracked().is_some() {
                // Viewing an older window – don't splice into it.
                new_below.update(|n| *n += 1);
                return;
            }
            let at_bottom = near_bottom(&scroller);
            let mine = message.author.as_ref().map(|a| a.id) == session.user_id();
            let mut m = message.clone();
            m.mentions_me = session.user.with_untracked(|u| {
                u.as_ref().is_some_and(|u| m.body_html.contains(&format!("href=\"/u/{}\"", u.username)))
            });
            apply.run(m);
            if let Some(a) = &message.author {
                typing.update(|t| {
                    t.remove(&a.id);
                });
            }
            if at_bottom || mine {
                leptos::task::spawn_local(async move {
                    sleep_ms(20).await;
                    scroll_to_bottom(&scroller);
                });
                mark_read();
            } else {
                new_below.update(|n| *n += 1);
            }
        }
        ServerMsg::ChatMessageUpdated { message } if message.channel_id == id => apply.run(message.clone()),
        ServerMsg::ChatMessageDeleted { channel_id, message_id } if *channel_id == id => {
            messages.update(|list| {
                if let Some(m) = list.iter_mut().find(|m| m.id == *message_id) {
                    m.deleted = true;
                    m.body_html.clear();
                    m.attachments.clear();
                    m.pinned = false;
                }
            });
        }
        ServerMsg::ChatReactions { channel_id, message_id, reactions } if *channel_id == id => {
            messages.update(|list| {
                if let Some(m) = list.iter_mut().find(|m| m.id == *message_id) {
                    let mine: Vec<String> = m.reactions.iter().filter(|r| r.me).map(|r| r.emoji.clone()).collect();
                    m.reactions = reactions
                        .iter()
                        .cloned()
                        .map(|mut r| {
                            r.me = mine.contains(&r.emoji);
                            r
                        })
                        .collect();
                }
            });
        }
        ServerMsg::ChatTyping { channel_id, user } if *channel_id == id => {
            typing.update(|t| {
                t.insert(user.id, (user.clone(), crate::state::now_ms()));
            });
        }
        ServerMsg::ChatChannelUpdated { channel_id } if *channel_id == id => load_detail(),
        _ => {}
    });
    // Expire typing indicators.
    #[cfg(feature = "hydrate")]
    {
        let handle = set_interval_with_handle(
            move || {
                let now = crate::state::now_ms();
                if typing.with_untracked(|t| t.values().any(|(_, ts)| now - ts > 5000.0)) {
                    typing.update(|t| t.retain(|_, (_, ts)| now - *ts <= 5000.0));
                }
            },
            std::time::Duration::from_secs(1),
        );
        on_cleanup(move || {
            if let Ok(h) = handle {
                h.clear();
            }
        });
    }

    let load_older = move |_| {
        let Some(c) = older.get_untracked() else { return };
        if loading_older.get_untracked() {
            return;
        }
        loading_older.set(true);
        let before_height = scroller.get_untracked().map(|d| d.scroll_height()).unwrap_or(0);
        leptos::task::spawn_local(async move {
            if let Ok(page) = list_messages(id, Some(c), None, None).await {
                messages.update(|list| {
                    let mut new = page.messages;
                    new.retain(|m| !list.iter().any(|x| x.id == m.id));
                    new.append(list);
                    *list = new;
                });
                older.set(page.older_cursor);
                sleep_ms(10).await;
                if let Some(d) = scroller.get_untracked() {
                    d.set_scroll_top(d.scroll_top() + d.scroll_height() - before_height);
                }
            }
            loading_older.set(false);
        });
    };
    let jump_latest = move |_| {
        new_below.set(0);
        highlight.set(None);
        if newer.get_untracked().is_some() {
            load_initial(None);
        } else {
            scroll_to_bottom(&scroller);
        }
        mark_read();
    };
    let on_scroll = move |_| {
        if near_bottom(&scroller) && new_below.get_untracked() > 0 && newer.get_untracked().is_none() {
            new_below.set(0);
            mark_read();
        }
        if let Some(d) = scroller.get_untracked() {
            if d.scroll_top() < 200 && older.get_untracked().is_some() && !loading_older.get_untracked() && loaded.get_untracked() {
                load_older(());
            }
        }
    };

    provide_context(PaneCtx {
        channel_id: id,
        detail,
        reply_to,
        editing,
        apply,
        highlight,
    });

    let leave = Callback::new(move |_: ()| {
        if !confirm("Diesen Kanal verlassen?") {
            return;
        }
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match leave_channel(id).await {
                Ok(()) => {
                    chat.reload_channels.run(());
                    navigate("/chat", Default::default());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    });
    let toggle_mute = move |_| {
        let muted = detail.with_untracked(|d| d.as_ref().map(|d| d.channel.muted).unwrap_or(false));
        leptos::task::spawn_local(async move {
            if mute_channel(id, !muted).await.is_ok() {
                detail.update(|d| {
                    if let Some(d) = d {
                        d.channel.muted = !muted;
                    }
                });
                chat.reload_channels.run(());
            }
        });
    };
    let join = move |_| {
        leptos::task::spawn_local(async move {
            if join_channel(id).await.is_ok() {
                load_detail();
                chat.reload_channels.run(());
            }
        });
    };

    view! {
        <section class="chat-pane">
            {move || detail_error.get().map(|e| view! { <div class="pad"><div class="alert alert-error">{e}</div></div> })}
            <header class="chat-header">
                <a class="icon-btn back-btn" href="/chat" aria-label="Zurück"><Icon name="arrow-left"/></a>
                {move || detail.get().map(|d| {
                    let c = d.channel.clone();
                    view! {
                        <Title text=c.display_name()/>
                        <div class="chat-title">
                            <h2><Icon name=c.kind.icon() size=18/>{c.name.clone()}</h2>
                            {(!c.topic.is_empty()).then(|| view! { <span class="muted small chat-topic">{c.topic.clone()}</span> })}
                        </div>
                    }
                })}
                <div class="chat-tools">
                    <button class="icon-btn" title="Mitglieder" class:active=move || panel.get() == Some("members") on:click=move |_| panel.update(|p| *p = if *p == Some("members") { None } else { Some("members") })>
                        <Icon name="users"/>
                    </button>
                    <button class="icon-btn" title="Angepinnt" class:active=move || panel.get() == Some("pins") on:click=move |_| panel.update(|p| *p = if *p == Some("pins") { None } else { Some("pins") })>
                        <Icon name="pin"/>
                    </button>
                    <button class="icon-btn" title="Suchen" class:active=move || panel.get() == Some("search") on:click=move |_| panel.update(|p| *p = if *p == Some("search") { None } else { Some("search") })>
                        <Icon name="search"/>
                    </button>
                    {crate::pages::chat_ext::header_actions(id, detail)}
                    <crate::components::Dropdown align_right=true label="Mehr" trigger=|| view! { <span class="icon-btn"><Icon name="more-vertical"/></span> }>
                        <button class="menu-item" on:click=toggle_mute>
                            {move || if detail.with(|d| d.as_ref().is_some_and(|d| d.channel.muted)) {
                                view! { <Icon name="volume" size=15/>"Stummschaltung aufheben" }.into_any()
                            } else {
                                view! { <Icon name="volume-x" size=15/>"Stummschalten" }.into_any()
                            }}
                        </button>
                        <Show when=move || detail.with(|d| d.as_ref().is_some_and(|d| d.can_manage))>
                            <button class="menu-item" on:click=move |_| settings_open.set(true)><Icon name="sliders" size=15/>"Einstellungen"</button>
                        </Show>
                        <button class="menu-item" on:click=move |_| { copy_to_clipboard(&format!("{}/chat/{id}", origin())); toasts.success("Link kopiert."); }><Icon name="link" size=15/>"Link kopieren"</button>
                        <Show when=move || detail.with(|d| d.as_ref().is_some_and(|d| d.channel.joined))>
                            <button class="menu-item danger" on:click=move |_| leave.run(())><Icon name="log-out" size=15/>"Verlassen"</button>
                        </Show>
                    </crate::components::Dropdown>
                </div>
            </header>
            <div class="chat-body">
                <div class="chat-main">
                    <div class="messages" node_ref=scroller on:scroll=on_scroll>
                        <Show when=move || older.with(|o| o.is_some())>
                            <div class="older-loader">
                                {move || if loading_older.get() { view! { <Spinner/> }.into_any() } else {
                                    view! { <button class="btn btn-sm btn-ghost" on:click=move |_| load_older(())>"Ältere Nachrichten laden"</button> }.into_any()
                                }}
                            </div>
                        </Show>
                        <Show when=move || loaded.get() fallback=|| view! { <Spinner/> }>
                            <Show when=move || older.with(|o| o.is_none())>
                                <div class="channel-start">
                                    {move || detail.get().map(|d| view! {
                                        <h3>{d.channel.display_name()}</h3>
                                        <p class="muted small">"Hier beginnt die Unterhaltung."</p>
                                    })}
                                </div>
                            </Show>
                            <For
                                each=move || {
                                    let list = messages.get();
                                    let lr = last_read.get();
                                    let me = session.user_id();
                                    let mut out = Vec::with_capacity(list.len());
                                    let mut prev: Option<&ChatMessageView> = None;
                                    let mut new_line_done = false;
                                    for m in &list {
                                        let day_break = prev.is_none_or(|p| local_date(p.created_at) != local_date(m.created_at));
                                        let grouped = !day_break && prev.is_some_and(|p| {
                                            p.kind == MessageKind::Text && m.kind == MessageKind::Text
                                                && p.author.as_ref().map(|a| a.id) == m.author.as_ref().map(|a| a.id)
                                                && (m.created_at - p.created_at).num_minutes() < 5
                                                && m.reply_to.is_none()
                                        });
                                        let new_line = !new_line_done
                                            && lr.is_some_and(|t| m.created_at > t)
                                            && m.author.as_ref().map(|a| a.id) != me;
                                        if new_line { new_line_done = true; }
                                        out.push((m.clone(), day_break, grouped, new_line));
                                        prev = Some(m);
                                    }
                                    out
                                }
                                key=|(m, day, grouped, new_line)| (m.id, m.edited_at, m.deleted, m.pinned, m.reactions.iter().map(|r| r.count).sum::<i64>(), *day, *grouped, *new_line, m.attachments.len())
                                children=move |(m, day_break, grouped, new_line)| view! {
                                    {day_break.then(|| view! { <div class="day-sep"><span>{local_date(m.created_at)}</span></div> })}
                                    {new_line.then(|| view! { <div class="new-sep"><span>"Neue Nachrichten"</span></div> })}
                                    <MessageRow m=m grouped=grouped/>
                                }
                            />
                        </Show>
                    </div>
                    <Show when=move || { new_below.get() > 0 }>
                        <button class="new-msgs-btn" on:click=jump_latest>
                            <Icon name="arrow-down" size=15/>{move || format!("{} neue Nachrichten", new_below.get())}
                        </button>
                    </Show>
                    <div class="typing-line small muted">
                        {move || {
                            let names: Vec<String> = typing.with(|t| t.values().map(|(u, _)| u.display_name.clone()).collect());
                            match names.len() {
                                0 => String::new(),
                                1 => format!("{} schreibt …", names[0]),
                                2 => format!("{} und {} schreiben …", names[0], names[1]),
                                n => format!("{n} Personen schreiben …"),
                            }
                        }}
                    </div>
                    {move || detail.get().map(|d| {
                        if d.channel.is_archived {
                            view! { <div class="composer-blocked muted"><Icon name="archive" size=16/>"Dieser Kanal ist archiviert."</div> }.into_any()
                        } else if !d.can_post {
                            view! { <div class="composer-blocked muted"><Icon name="lock" size=16/>"Du kannst hier nicht schreiben."</div> }.into_any()
                        } else if !d.channel.joined && d.channel.kind == ChannelKind::Public {
                            view! {
                                <div class="composer-blocked">
                                    <span class="muted">"Du siehst dir #"{d.channel.name.clone()}" an."</span>
                                    <button class="btn btn-primary btn-sm" on:click=join>"Beitreten"</button>
                                </div>
                            }.into_any()
                        } else {
                            view! { <Composer channel_id=id/> }.into_any()
                        }
                    })}
                </div>
                {move || panel.get().map(|p| match p {
                    "members" => view! { <MembersPanel/> }.into_any(),
                    "pins" => view! { <PinsPanel/> }.into_any(),
                    _ => view! { <SearchPanel/> }.into_any(),
                })}
            </div>
            <ChannelSettingsModal open=settings_open/>
        </section>
    }
}

fn chat_unread_signal() -> Option<RwSignal<i64>> {
    use_context::<ChatUnread>().map(|c| c.0)
}

fn refresh_chat_badge_with(sig: Option<RwSignal<i64>>) {
    if let Some(u) = sig {
        leptos::task::spawn_local(async move {
            if let Ok(n) = chat_unread_total().await {
                u.set(n);
            }
        });
    }
}

fn scroll_to_message(id: Uuid) {
    #[cfg(feature = "hydrate")]
    if let Some(el) = document().get_element_by_id(&format!("msg-{id}")) {
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Center);
        el.scroll_into_view_with_scroll_into_view_options(&opts);
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = id;
}

#[component]
fn MessageRow(m: ChatMessageView, grouped: bool) -> impl IntoView {
    let pane = expect_context::<PaneCtx>();
    let session = use_session();
    let toasts = use_toasts();
    let id = m.id;
    if m.kind == MessageKind::System {
        return view! {
            <div class="msg-system" id=format!("msg-{id}")>
                <span inner_html=m.body_html.clone()></span>
                <span class="msg-time">{local_time(m.created_at)}</span>
            </div>
        }
        .into_any();
    }
    let author_id = m.author.as_ref().map(|a| a.id);
    let mine = author_id.is_some() && author_id == session.user_id();
    let can_moderate = move || pane.detail.with(|d| d.as_ref().is_some_and(|d| d.can_moderate));
    let can_pin = move || pane.detail.with(|d| d.as_ref().is_some_and(|d| d.can_moderate || (d.channel.kind.is_dm() && d.channel.joined)));
    let edit_body = RwSignal::new(String::new());
    let is_editing = move || pane.editing.get() == Some(id);
    let highlighted = move || pane.highlight.get() == Some(id);

    let start_edit = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(src) = message_source(id).await {
                edit_body.set(src);
                pane.editing.set(Some(id));
            }
        });
    };
    let save_edit = move || {
        let body = edit_body.get_untracked();
        leptos::task::spawn_local(async move {
            match edit_message(id, body).await {
                Ok(m) => {
                    pane.apply.run(m);
                    pane.editing.set(None);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let remove = move |_| {
        if !confirm("Nachricht löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            if let Err(e) = delete_message(id).await {
                toasts.error(e.to_string());
            }
        });
    };
    let pinned = m.pinned;
    let toggle_pin = move |_| {
        leptos::task::spawn_local(async move {
            match pin_message(id, !pinned).await {
                Ok(m) => pane.apply.run(m),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let reply_target = m.clone();
    let channel_id = pane.channel_id;

    view! {
        <div class="msg" class:grouped=grouped class:mention=m.mentions_me class:highlight=highlighted class:pinned=m.pinned id=format!("msg-{id}")>
            <div class="msg-gutter">
                {if grouped {
                    view! { <span class="msg-time-hover">{local_time(m.created_at)}</span> }.into_any()
                } else {
                    m.author.clone().map(|a| { let href = a.profile_url(); view! { <a href=href><Avatar user=a size=36/></a> }.into_any() })
                        .unwrap_or_else(|| view! { <span class="avatar-placeholder"></span> }.into_any())
                }}
            </div>
            <div class="msg-main">
                {(!grouped).then(|| view! {
                    <div class="msg-head">
                        {m.author.clone().map(|a| view! { <a class="user-link" href=a.profile_url()>{a.display_name.clone()}</a> }.into_any())
                            .unwrap_or_else(|| view! { <span class="muted">"Gelöschter Benutzer"</span> }.into_any())}
                        <span class="msg-time" title=crate::components::common::local_datetime(m.created_at)>{local_time(m.created_at)}</span>
                        {m.pinned.then(|| view! { <span class="msg-flag" title="Angepinnt"><Icon name="pin" size=12/></span> })}
                    </div>
                })}
                {m.reply_to.clone().map(|r| {
                    let target = r.id;
                    view! {
                        <button class="msg-reply-ref" on:click=move |_| { pane.highlight.set(Some(target)); scroll_to_message(target); }>
                            <Icon name="corner-down-right" size=12/><strong>{r.author_name.clone()}</strong>" "{r.excerpt.clone()}
                        </button>
                    }
                })}
                {move || if is_editing() {
                    view! {
                        <div class="msg-edit">
                            <textarea rows=2 prop:value=move || edit_body.get() on:input=move |ev| edit_body.set(event_target_value(&ev))
                                on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                    if ev.key() == "Enter" && !ev.shift_key() { ev.prevent_default(); save_edit(); }
                                    if ev.key() == "Escape" { pane.editing.set(None); }
                                }></textarea>
                            <div class="small muted">"Enter speichert · Esc bricht ab"</div>
                        </div>
                    }.into_any()
                } else if m.deleted {
                    view! { <div class="msg-deleted muted"><Icon name="trash" size=13/>"Nachricht gelöscht"</div> }.into_any()
                } else {
                    view! {
                        <div class="msg-body prose" inner_html=m.body_html.clone()></div>
                        {m.edited_at.map(|_| view! { <span class="msg-edited muted">"(bearbeitet)"</span> })}
                    }.into_any()
                }}
                {(!m.attachments.is_empty()).then(|| view! { <Attachments files=m.attachments.clone()/> })}
                {(!m.deleted && !m.reactions.is_empty()).then(|| view! {
                    <ReactionBar subject_type=SubjectType::ChatMessage subject_id=id reactions=m.reactions.clone()/>
                })}
            </div>
            {(!m.deleted).then(|| view! {
                <div class="msg-actions">
                    <ReactionBar subject_type=SubjectType::ChatMessage subject_id=id reactions=Vec::new()/>
                    <button class="action-btn" title="Antworten" on:click=move |_| pane.reply_to.set(Some(reply_target.clone()))><Icon name="reply" size=15/></button>
                    <Show when=move || mine>
                        <button class="action-btn" title="Bearbeiten" on:click=start_edit><Icon name="edit" size=15/></button>
                    </Show>
                    <Show when=can_pin>
                        <button class="action-btn" title=if pinned { "Lösen" } else { "Anpinnen" } on:click=toggle_pin><Icon name="pin" size=15/></button>
                    </Show>
                    <button class="action-btn" title="Link kopieren" on:click=move |_| { copy_to_clipboard(&format!("{}/chat/{channel_id}?m={id}", origin())); toasts.success("Link kopiert."); }><Icon name="link" size=15/></button>
                    <Show when=move || mine || can_moderate()>
                        <button class="action-btn" title="Löschen" on:click=remove><Icon name="trash" size=15/></button>
                    </Show>
                    <Show when=move || !mine>
                        <ReportButton subject_type=SubjectType::ChatMessage subject_id=id compact=true/>
                    </Show>
                </div>
            })}
        </div>
    }
    .into_any()
}

#[component]
fn Attachments(files: Vec<FileView>) -> impl IntoView {
    let (images, others): (Vec<FileView>, Vec<FileView>) = files.into_iter().partition(|f| f.kind() == FileKind::Image && f.has_thumb);
    view! {
        {(!images.is_empty()).then(|| view! {
            <div class="msg-images">
                {images.into_iter().map(|f| view! {
                    <a href=f.url() target="_blank" rel="external"><img src=f.preview_url() alt=f.name.clone() loading="lazy"/></a>
                }).collect_view()}
            </div>
        })}
        {(!others.is_empty()).then(|| view! {
            <div class="msg-files">
                {others.into_iter().map(|f| {
                    let video = f.kind() == FileKind::Video;
                    view! {
                        <div class="msg-file">
                            {if video {
                                view! { <video src=f.url() controls=true preload="metadata"></video> }.into_any()
                            } else {
                                view! {
                                    <FileThumb file=f.clone() size=36/>
                                    <div class="grow">
                                        <a href=f.url() target="_blank" rel="external" class="ellipsis">{f.name.clone()}</a>
                                        <div class="muted small">{shared::util::format_bytes(f.size_bytes)}</div>
                                    </div>
                                    <a class="icon-btn" href=f.download_url() rel="external" download=""><Icon name="download" size=16/></a>
                                }.into_any()
                            }}
                        </div>
                    }
                }).collect_view()}
            </div>
        })}
    }
}

#[derive(Clone)]
struct PendingFile {
    key: u64,
    name: String,
    progress: RwSignal<f64>,
    file: RwSignal<Option<FileView>>,
    error: RwSignal<Option<String>>,
}

#[component]
fn Composer(channel_id: Uuid) -> impl IntoView {
    let pane = expect_context::<PaneCtx>();
    let rt = use_realtime();
    let toasts = use_toasts();
    let body = RwSignal::new(String::new());
    let sending = RwSignal::new(false);
    let pending = RwSignal::new(Vec::<PendingFile>::new());
    let next_key = RwSignal::new(0u64);
    let last_typing = RwSignal::new(0.0f64);
    let textarea = NodeRef::<html::Textarea>::new();
    let suggestions = RwSignal::new(Vec::<UserSummary>::new());
    let mention = RwSignal::new(None::<(u32, String)>);

    let uploading = move || pending.with(|p| p.iter().any(|f| f.file.get().is_none() && f.error.get().is_none()));
    let send = move || {
        let text = body.get_untracked();
        let attachments: Vec<Uuid> = pending.with_untracked(|p| p.iter().filter_map(|f| f.file.get_untracked().map(|f| f.id)).collect());
        if (text.trim().is_empty() && attachments.is_empty()) || sending.get_untracked() || uploading() {
            return;
        }
        sending.set(true);
        let reply = pane.reply_to.get_untracked().map(|m| m.id);
        leptos::task::spawn_local(async move {
            match send_message(channel_id, text, reply, attachments).await {
                Ok(m) => {
                    body.set(String::new());
                    pending.set(Vec::new());
                    pane.reply_to.set(None);
                    pane.apply.run(m);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            sending.set(false);
            if let Some(el) = textarea.get_untracked() {
                let _ = el.focus();
            }
        });
    };
    let choose = move |u: UserSummary| {
        #[cfg(feature = "hydrate")]
        if let (Some(el), Some((start, q))) = (textarea.get(), mention.get_untracked()) {
            let end = start + 1 + q.encode_utf16().count() as u32;
            crate::components::markdown_editor::splice(&el, start, end, &format!("@{} ", u.username));
            body.set(el.value());
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = u;
        suggestions.set(Vec::new());
        mention.set(None);
    };
    let on_input = move |ev: leptos::ev::Event| {
        body.set(event_target_value(&ev));
        let now = crate::state::now_ms();
        if now - last_typing.get_untracked() > 3000.0 && !body.with_untracked(|b| b.is_empty()) {
            last_typing.set(now);
            rt.send(&ClientMsg::Typing { channel_id });
        }
        #[cfg(feature = "hydrate")]
        if let Some(el) = textarea.get() {
            match crate::components::markdown_editor::current_mention(&el) {
                Some((start, q)) if !q.is_empty() => {
                    mention.set(Some((start, q.clone())));
                    leptos::task::spawn_local(async move {
                        sleep_ms(150).await;
                        if mention.get_untracked().map(|(_, x)| x) == Some(q.clone()) {
                            if let Ok(list) = crate::api::users::mention_suggest(q).await {
                                suggestions.set(list);
                            }
                        }
                    });
                }
                _ => {
                    mention.set(None);
                    suggestions.set(Vec::new());
                }
            }
        }
    };
    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if !suggestions.with_untracked(|s| s.is_empty()) && (ev.key() == "Tab" || ev.key() == "Enter") {
            ev.prevent_default();
            if let Some(u) = suggestions.get_untracked().first().cloned() {
                choose(u);
            }
            return;
        }
        if ev.key() == "Escape" {
            suggestions.set(Vec::new());
            pane.reply_to.set(None);
        }
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            send();
        }
    };
    let add_files = move |files: Vec<web_sys::File>| {
        #[cfg(feature = "hydrate")]
        for f in files {
            let key = next_key.get_untracked();
            next_key.set(key + 1);
            let item = PendingFile {
                key,
                name: f.name(),
                progress: RwSignal::new(0.0),
                file: RwSignal::new(None),
                error: RwSignal::new(None),
            };
            pending.update(|p| p.push(item.clone()));
            leptos::task::spawn_local(async move {
                let opts = crate::upload::UploadOpts {
                    purpose: shared::files::FilePurpose::Chat,
                    context_id: Some(channel_id),
                    ..Default::default()
                };
                let progress = item.progress;
                match crate::upload::upload_file(f, opts, move |p| progress.set(p)).await {
                    Ok(res) => match (res.files.into_iter().next(), res.errors.into_iter().next()) {
                        (Some(file), _) => item.file.set(Some(file)),
                        (None, Some(e)) => item.error.set(Some(e)),
                        _ => item.error.set(Some("Upload fehlgeschlagen".into())),
                    },
                    Err(e) => item.error.set(Some(e)),
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = (files, next_key);
    };
    let on_pick = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        add_files(crate::upload::files_from_input(&ev));
        #[cfg(not(feature = "hydrate"))]
        let _ = (ev, add_files);
    };
    let on_paste = move |ev: leptos::ev::ClipboardEvent| {
        #[cfg(feature = "hydrate")]
        {
            {
                if let Some(list) = ev.clipboard_data().and_then(|d| d.files()) {
                    let mut files = Vec::new();
                    for i in 0..list.length() {
                        if let Some(f) = list.get(i) {
                            files.push(f);
                        }
                    }
                    if !files.is_empty() {
                        ev.prevent_default();
                        add_files(files);
                    }
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };
    let on_drop = move |ev: leptos::ev::DragEvent| {
        ev.prevent_default();
        #[cfg(feature = "hydrate")]
        add_files(crate::upload::files_from_drop(&ev));
    };

    view! {
        <div class="composer-chat" on:dragover=|ev: leptos::ev::DragEvent| ev.prevent_default() on:drop=on_drop>
            {move || pane.reply_to.get().map(|r| view! {
                <div class="reply-bar">
                    <Icon name="corner-down-right" size=14/>
                    <span class="grow ellipsis">"Antwort an "<strong>{r.author.map(|a| a.display_name).unwrap_or_default()}</strong></span>
                    <button class="icon-btn" on:click=move |_| pane.reply_to.set(None)><Icon name="x" size=14/></button>
                </div>
            })}
            <Show when=move || !pending.with(|p| p.is_empty())>
                <div class="pending-files">
                    <For each=move || pending.get() key=|p| p.key children=move |p| {
                        let key = p.key;
                        let (progress, file, error) = (p.progress, p.file, p.error);
                        view! {
                            <div class="pending-file" class:failed=move || error.get().is_some()>
                                <Icon name="paperclip" size=13/>
                                <span class="ellipsis">{p.name.clone()}</span>
                                {move || if let Some(e) = error.get() {
                                    view! { <span class="error small" title=e.clone()>"Fehler"</span> }.into_any()
                                } else if file.get().is_some() {
                                    view! { <Icon name="check" size=13/> }.into_any()
                                } else {
                                    view! { <span class="small muted">{move || format!("{:.0}%", progress.get() * 100.0)}</span> }.into_any()
                                }}
                                <button class="icon-btn" on:click=move |_| pending.update(|v| v.retain(|x| x.key != key))><Icon name="x" size=12/></button>
                            </div>
                        }
                    }/>
                </div>
            </Show>
            <div class="composer-row">
                <label class="icon-btn" title="Datei anhängen">
                    <Icon name="paperclip"/>
                    <input type="file" multiple class="hidden" on:change=on_pick/>
                </label>
                <div class="composer-input">
                    <textarea node_ref=textarea rows=1 placeholder=move || {
                        pane.detail.with(|d| d.as_ref().map(|d| format!("Nachricht an {}", d.channel.display_name())).unwrap_or_default())
                    }
                        prop:value=move || body.get() on:input=on_input on:keydown=on_keydown on:paste=on_paste></textarea>
                    <Show when=move || !suggestions.with(|s| s.is_empty())>
                        <ul class="mention-list">
                            <For each=move || suggestions.get() key=|u| u.id children=move |u| {
                                let pick = u.clone();
                                view! {
                                    <li on:mousedown=move |ev| { ev.prevent_default(); choose(pick.clone()); }>
                                        <Avatar user=u.clone() size=20/><span>{u.display_name.clone()}</span><span class="muted">"@"{u.username.clone()}</span>
                                    </li>
                                }
                            }/>
                        </ul>
                    </Show>
                </div>
                <button class="btn btn-primary send-btn" title="Senden" disabled=move || sending.get() || uploading() on:click=move |_| send()>
                    <Icon name="send" size=16/>
                </button>
            </div>
            <div class="composer-hint small muted">"Enter sendet · Shift+Enter neue Zeile · Markdown · Dateien per Drag & Drop oder Einfügen"</div>
        </div>
    }
}

#[component]
fn MembersPanel() -> impl IntoView {
    let pane = expect_context::<PaneCtx>();
    let rt = use_realtime();
    let session = use_session();
    let toasts = use_toasts();
    let adding = RwSignal::new(Vec::<UserSummary>::new());
    let can_add = move || pane.detail.with(|d| d.as_ref().is_some_and(|d| d.can_manage || d.channel.kind == ChannelKind::Group));
    let add = move |_| {
        let ids: Vec<Uuid> = adding.get_untracked().iter().map(|u| u.id).collect();
        if ids.is_empty() {
            return;
        }
        let channel_id = pane.channel_id;
        leptos::task::spawn_local(async move {
            match add_channel_members(channel_id, ids).await {
                Ok(()) => {
                    adding.set(Vec::new());
                    if let Ok(d) = get_channel_detail(channel_id).await {
                        pane.detail.set(Some(d));
                    }
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <aside class="chat-panel">
            <h3>"Mitglieder"</h3>
            <Show when=can_add>
                <div class="stack-sm add-members">
                    <UserPicker selected=adding placeholder="Mitglied hinzufügen …"/>
                    <Show when=move || !adding.with(|a| a.is_empty())>
                        <button class="btn btn-sm btn-primary" on:click=add>"Hinzufügen"</button>
                    </Show>
                </div>
            </Show>
            <ul class="member-list">
                {move || pane.detail.get().map(|d| {
                    let can_manage = d.can_manage;
                    let channel_id = d.channel.id;
                    let i_am_owner = d.channel.my_role == Some(ChatRole::Owner);
                    let mut members = d.members.clone();
                    members.sort_by_key(|m| (!rt.is_online(m.user.id), m.user.display_name.to_lowercase()));
                    members.into_iter().map(|m| {
                        let uid = m.user.id;
                        let me = session.user_id() == Some(uid);
                        view! {
                            <li>
                                <Avatar user=m.user.clone() size=26 presence=true/>
                                <a class="grow ellipsis" href=m.user.profile_url()>{m.user.display_name.clone()}</a>
                                {(m.role != ChatRole::Member).then(|| view! { <span class="badge">{m.role.label()}</span> })}
                                {(i_am_owner && !me && m.role != ChatRole::Owner).then(|| {
                                    let is_admin = m.role == ChatRole::Admin;
                                    view! {
                                        <button class="icon-btn" title=if is_admin { "Admin-Rechte entziehen" } else { "Zum Admin machen" } on:click=move |_| {
                                            leptos::task::spawn_local(async move {
                                                let role = if is_admin { ChatRole::Member } else { ChatRole::Admin };
                                                if set_channel_member_role(channel_id, uid, role).await.is_ok() {
                                                    if let Ok(d) = get_channel_detail(channel_id).await { pane.detail.set(Some(d)); }
                                                }
                                            });
                                        }><Icon name="shield" size=14/></button>
                                    }
                                })}
                                {(can_manage && !me && m.role != ChatRole::Owner).then(|| view! {
                                    <button class="icon-btn" title="Entfernen" on:click=move |_| {
                                        if !confirm("Mitglied entfernen?") { return; }
                                        leptos::task::spawn_local(async move {
                                            match remove_channel_member(channel_id, uid).await {
                                                Ok(()) => { if let Ok(d) = get_channel_detail(channel_id).await { pane.detail.set(Some(d)); } }
                                                Err(e) => toasts.error(e.to_string()),
                                            }
                                        });
                                    }><Icon name="user-x" size=14/></button>
                                })}
                            </li>
                        }
                    }).collect_view()
                })}
            </ul>
        </aside>
    }
}

#[component]
fn PinsPanel() -> impl IntoView {
    let pane = expect_context::<PaneCtx>();
    let pins = Resource::new(move || pane.channel_id, pinned_messages);
    view! {
        <aside class="chat-panel">
            <h3>"Angepinnt"</h3>
            <Suspense fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    match pins.await {
                        Ok(list) if list.is_empty() => view! { <p class="muted small">"Noch nichts angepinnt."</p> }.into_any(),
                        Ok(list) => view! { <PanelMessages list=list/> }.into_any(),
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    }
                })}
            </Suspense>
        </aside>
    }
}

#[component]
fn SearchPanel() -> impl IntoView {
    let pane = expect_context::<PaneCtx>();
    let q = RwSignal::new(String::new());
    let results = RwSignal::new(None::<Vec<ChatMessageView>>);
    let run = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let query = q.get_untracked();
        let channel_id = pane.channel_id;
        leptos::task::spawn_local(async move {
            if let Ok(list) = search_messages(channel_id, query).await {
                results.set(Some(list));
            }
        });
    };
    view! {
        <aside class="chat-panel">
            <h3>"Suchen"</h3>
            <form on:submit=run><input type="search" placeholder="In diesem Kanal suchen …" bind:value=q autofocus=true/></form>
            {move || results.get().map(|list| if list.is_empty() {
                view! { <p class="muted small">"Keine Treffer."</p> }.into_any()
            } else {
                view! { <PanelMessages list=list/> }.into_any()
            })}
        </aside>
    }
}

#[component]
fn PanelMessages(list: Vec<ChatMessageView>) -> impl IntoView {
    let pane = expect_context::<PaneCtx>();
    let navigate = use_navigate();
    view! {
        <ul class="panel-messages">
            {list.into_iter().map(|m| {
                let id = m.id;
                let navigate = navigate.clone();
                let channel_id = pane.channel_id;
                view! {
                    <li on:click=move |_| {
                        pane.highlight.set(Some(id));
                        if document_has(id) {
                            scroll_to_message(id);
                        } else {
                            navigate(&format!("/chat/{channel_id}?m={id}"), Default::default());
                        }
                    }>
                        <div class="row gap-sm small">
                            {m.author.clone().map(|a| view! { <strong>{a.display_name}</strong> })}
                            <span class="muted"><TimeAgo ts=m.created_at/></span>
                        </div>
                        <div class="prose small panel-msg-body" inner_html=m.body_html.clone()></div>
                    </li>
                }
            }).collect_view()}
        </ul>
    }
}

fn document_has(id: Uuid) -> bool {
    #[cfg(feature = "hydrate")]
    {
        document().get_element_by_id(&format!("msg-{id}")).is_some()
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = id;
        false
    }
}

#[component]
fn ChannelSettingsModal(open: RwSignal<bool>) -> impl IntoView {
    let pane = expect_context::<PaneCtx>();
    let chat = expect_context::<ChatCtx>();
    let toasts = use_toasts();
    let name = RwSignal::new(String::new());
    let topic = RwSignal::new(String::new());
    Effect::new(move |_| {
        if open.get() {
            if let Some(d) = pane.detail.get_untracked() {
                name.set(d.channel.name.clone());
                topic.set(d.channel.topic.clone());
            }
        }
    });
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let id = pane.channel_id;
        leptos::task::spawn_local(async move {
            match update_channel(id, name.get_untracked(), topic.get_untracked()).await {
                Ok(()) => {
                    open.set(false);
                    chat.reload_channels.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let archive = move |_| {
        let id = pane.channel_id;
        let archived = pane.detail.with_untracked(|d| d.as_ref().is_some_and(|d| d.channel.is_archived));
        leptos::task::spawn_local(async move {
            match archive_channel(id, !archived).await {
                Ok(()) => {
                    open.set(false);
                    chat.reload_channels.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Modal open=open title="Kanal-Einstellungen">
            <form class="stack" on:submit=save>
                <label class="field"><span>"Name"</span><input type="text" bind:value=name/></label>
                <label class="field"><span>"Thema"</span><input type="text" bind:value=topic/></label>
                <div class="row gap">
                    <Show when=move || pane.detail.with(|d| d.as_ref().is_some_and(|d| !d.channel.kind.is_dm()))>
                        <button type="button" class="btn btn-danger-soft" on:click=archive>
                            <Icon name="archive" size=16/>
                            {move || if pane.detail.with(|d| d.as_ref().is_some_and(|d| d.channel.is_archived)) { "Wiederherstellen" } else { "Archivieren" }}
                        </button>
                    </Show>
                    <span class="grow"></span>
                    <button class="btn btn-primary">"Speichern"</button>
                </div>
            </form>
        </Modal>
    }
}
