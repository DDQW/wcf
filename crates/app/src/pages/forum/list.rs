use leptos::prelude::*;
use shared::forum::{TopicFilter, TopicListItem, TopicSort};

use crate::api::forum::list_topics;
use crate::components::{Avatar, EmptyState, ErrorBox, Icon, InfiniteSentinel, Spinner, TimeAgo};
use crate::state::use_session;

#[component]
pub fn CategoryChip(#[prop(into)] name: String, #[prop(into)] color: String, id: uuid::Uuid) -> impl IntoView {
    view! {
        <a class="cat-chip" href=format!("/forum/c/{id}")>
            <span class="cat-dot" style=format!("background:{color}")></span>{name}
        </a>
    }
}

#[component]
pub fn TagChip(#[prop(into)] tag: String) -> impl IntoView {
    view! { <a class="tag-chip" href=format!("/forum/tag/{tag}")>"#"{tag.clone()}</a> }
}

#[component]
pub fn TopicRow(topic: TopicListItem, #[prop(optional)] compact: bool) -> impl IntoView {
    let session = use_session();
    let t = topic;
    let recent = chrono::Utc::now() - t.created_at < chrono::Duration::days(7);
    let unread = t.unread_count;
    let href = match unread {
        Some(n) if n > 0 => format!("/forum/t/{}?unread=1", t.id),
        _ => format!("/forum/t/{}", t.id),
    };
    view! {
        <div class="topic-row" class:compact=compact class:has-unread=move || session.logged_in() && unread.is_some_and(|n| n > 0)>
            <Avatar user=t.author.clone() size=if compact { 30 } else { 38 }/>
            <div class="topic-main">
                <div class="topic-title-line">
                    {t.is_pinned.then(|| view! { <span class="topic-flag" title="Angepinnt"><Icon name="pin" size=14/></span> })}
                    {t.is_locked.then(|| view! { <span class="topic-flag" title="Gesperrt"><Icon name="lock" size=14/></span> })}
                    <a class="topic-title" href=href>{t.title.clone()}</a>
                    {move || session.logged_in().then(|| match unread {
                        Some(n) if n > 0 => Some(view! { <span class="badge badge-new">{n} " neu"</span> }.into_any()),
                        None if recent => Some(view! { <span class="badge badge-new">"Neu"</span> }.into_any()),
                        _ => None,
                    })}
                </div>
                <div class="topic-meta">
                    <CategoryChip name=t.category.name.clone() color=t.category.color.clone() id=t.category.id/>
                    {t.tags.iter().map(|tag| view! { <TagChip tag=tag.clone()/> }).collect_view()}
                    <span class="muted small">{t.author.display_name.clone()} " · " <TimeAgo ts=t.created_at/></span>
                </div>
            </div>
            {(!compact).then(|| view! {
                <div class="topic-stats">
                    <span title="Antworten"><Icon name="message-circle" size=14/>{t.reply_count}</span>
                    <span title="Aufrufe"><Icon name="eye" size=14/>{t.view_count}</span>
                    <span title="Reaktionen"><Icon name="heart" size=14/>{t.like_count}</span>
                </div>
            })}
            <div class="topic-last">
                {t.last_poster.clone().map(|u| view! { <Avatar user=u size=22/> })}
                <span class="small muted"><TimeAgo ts=t.last_post_at/></span>
            </div>
        </div>
    }
}

/// Topic list with the first page rendered on the server and further pages
/// loaded by infinite scrolling.
#[component]
pub fn TopicList(
    #[prop(into)] filter: Signal<TopicFilter>,
    #[prop(into)] sort: Signal<TopicSort>,
    #[prop(optional, into)] empty_text: String,
) -> impl IntoView {
    let first = Resource::new(move || (filter.get(), sort.get()), |(f, s)| list_topics(f, s, None));
    let extra = RwSignal::new(Vec::<TopicListItem>::new());
    let cursor = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let has_more = Signal::derive(move || cursor.with(|c| c.is_some()));
    let empty_text = if empty_text.is_empty() { "Hier gibt es noch keine Themen.".to_string() } else { empty_text };

    Effect::new(move |_| {
        filter.track();
        sort.track();
        extra.set(Vec::new());
    });

    let load_more = move |_| {
        let Some(c) = cursor.get_untracked() else { return };
        loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(page) = list_topics(filter.get_untracked(), sort.get_untracked(), Some(c)).await {
                extra.update(|v| v.extend(page.items));
                cursor.set(page.next_cursor);
            }
            loading.set(false);
        });
    };

    view! {
        <Transition fallback=|| view! { <Spinner/> }>
            {move || {
                let empty_text = empty_text.clone();
                Suspend::new(async move {
                    match first.await {
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        Ok(page) if page.items.is_empty() => {
                            cursor.set(None);
                            view! { <EmptyState icon="message-square" title=empty_text/> }.into_any()
                        }
                        Ok(page) => {
                            cursor.set(page.next_cursor.clone());
                            view! {
                                <div class="topic-list card flush">
                                    {page.items.into_iter().map(|t| view! { <TopicRow topic=t/> }).collect_view()}
                                    <For each=move || extra.get() key=|t| t.id children=|t| view! { <TopicRow topic=t/> }/>
                                </div>
                                <InfiniteSentinel on_visible=load_more enabled=has_more loading=loading done_text="Keine weiteren Themen."/>
                            }.into_any()
                        }
                    }
                })
            }}
        </Transition>
    }
}
