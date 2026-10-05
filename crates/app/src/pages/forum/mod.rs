pub mod list;
mod thread;
mod topic;

pub use topic::TopicPage;

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use shared::forum::{CategoryView, NewTopic, TopicFilter, TopicSort};
use uuid::Uuid;

use crate::api::forum::{create_topic, forum_categories, forum_category, fetch_forum_overview};
use crate::components::{
    EmptyState, ErrorBox, Icon, MarkdownEditor, PageHeader, RequireLogin, Spinner, TimeAgo,
};
use crate::pages::not_found::Missing;
use crate::state::{use_session, use_toasts};
use list::{TagChip, TopicList};

fn sort_tabs(base: String, current: TopicSort) -> impl IntoView {
    view! {
        <div class="tabs">
            {[TopicSort::Activity, TopicSort::Newest, TopicSort::Top].into_iter().map(|s| view! {
                <a class="tab" class:active=current == s href=format!("{base}?sort={}", s.as_str())>{s.label()}</a>
            }).collect_view()}
        </div>
    }
}

#[component]
pub fn ForumIndex() -> impl IntoView {
    let session = use_session();
    let query = use_query_map();
    let view_mode = move || query.read().get("view").unwrap_or_default();
    view! {
        <Title text="Forum"/>
        <PageHeader title="Forum">
            <Show when=move || session.logged_in()>
                <a class="btn btn-primary" href="/forum/new"><Icon name="plus" size=16/>"Neues Thema"</a>
            </Show>
        </PageHeader>
        <div class="tabs">
            <a class="tab" class:active=move || view_mode().is_empty() href="/forum">"Kategorien"</a>
            <a class="tab" class:active=move || view_mode() == "latest" href="/forum?view=latest">"Neueste"</a>
            <Show when=move || session.logged_in()>
                <a class="tab" class:active=move || view_mode() == "unread" href="/forum?view=unread">"Ungelesen"</a>
                <a class="tab" class:active=move || view_mode() == "watching" href="/forum?view=watching">"Beobachtet"</a>
            </Show>
        </div>
        {move || match view_mode().as_str() {
            "latest" => view! { <TopicList filter=TopicFilter::All sort=TopicSort::Activity/> }.into_any(),
            "unread" => view! { <TopicList filter=TopicFilter::Unread sort=TopicSort::Activity empty_text="Alles gelesen!"/> }.into_any(),
            "watching" => view! { <TopicList filter=TopicFilter::Watching sort=TopicSort::Activity empty_text="Du beobachtest noch keine Themen."/> }.into_any(),
            _ => view! { <CategoryOverview/> }.into_any(),
        }}
    }
}

#[component]
fn CategoryOverview() -> impl IntoView {
    let data = Resource::new(|| (), |_| fetch_forum_overview());
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(o) if o.categories.is_empty() => view! {
                        <EmptyState icon="message-square" title="Noch keine Kategorien.">
                            <p class="muted">"Admins können unter Verwaltung → Forum Kategorien anlegen."</p>
                        </EmptyState>
                    }.into_any(),
                    Ok(o) => {
                        let all = o.categories.clone();
                        let parents: Vec<CategoryView> = o.categories.iter().filter(|c| c.parent_id.is_none()).cloned().collect();
                        view! {
                            <div class="forum-layout">
                                <div class="cat-list">
                                    {parents.into_iter().map(|c| {
                                        let children: Vec<CategoryView> = all.iter().filter(|x| x.parent_id == Some(c.id)).cloned().collect();
                                        view! { <CategoryCard c=c children=children/> }
                                    }).collect_view()}
                                </div>
                                <aside class="card">
                                    <h3><Icon name="tag" size=16/>"Beliebte Tags"</h3>
                                    {if o.popular_tags.is_empty() {
                                        view! { <p class="muted small">"Noch keine Tags."</p> }.into_any()
                                    } else {
                                        view! {
                                            <div class="tag-cloud">
                                                {o.popular_tags.into_iter().map(|t| view! {
                                                    <span class="tag-count-wrap"><TagChip tag=t.tag.clone()/><span class="muted small">{t.count}</span></span>
                                                }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                </aside>
                            </div>
                        }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}

#[component]
fn CategoryCard(c: CategoryView, children: Vec<CategoryView>) -> impl IntoView {
    let topics: i64 = c.topic_count + children.iter().map(|x| x.topic_count).sum::<i64>();
    let posts: i64 = c.post_count + children.iter().map(|x| x.post_count).sum::<i64>();
    let last = std::iter::once(c.last_topic.clone())
        .chain(children.iter().map(|x| x.last_topic.clone()))
        .flatten()
        .max_by_key(|t| t.last_post_at);
    view! {
        <article class="cat-card card" style=format!("--cat-color:{}", c.color)>
            <div class="cat-card-main">
                <a class="cat-name" href=format!("/forum/c/{}", c.id)>{c.name.clone()}</a>
                {c.description.clone().map(|d| view! { <p class="muted">{d}</p> })}
                {(!children.is_empty()).then(|| view! {
                    <div class="subcats">
                        {children.iter().map(|s| view! {
                            <a class="cat-chip" href=format!("/forum/c/{}", s.id)>
                                <span class="cat-dot" style=format!("background:{}", s.color)></span>{s.name.clone()}
                            </a>
                        }).collect_view()}
                    </div>
                })}
            </div>
            <div class="cat-card-stats">
                <div><strong>{topics}</strong><span class="muted small">"Themen"</span></div>
                <div><strong>{posts}</strong><span class="muted small">"Beiträge"</span></div>
            </div>
            <div class="cat-card-last">
                {match last {
                    Some(t) => view! {
                        <a href=format!("/forum/t/{}", t.id) class="small">{t.title.clone()}</a>
                        <span class="muted small">
                            {t.last_poster.map(|u| u.display_name)} " · " <TimeAgo ts=t.last_post_at/>
                        </span>
                    }.into_any(),
                    None => view! { <span class="muted small">"Noch keine Themen"</span> }.into_any(),
                }}
            </div>
        </article>
    }
}

#[component]
pub fn CategoryPage() -> impl IntoView {
    let session = use_session();
    let params = use_params_map();
    let query = use_query_map();
    let id = move || params.read().get("id").and_then(|s| Uuid::parse_str(&s).ok());
    let sort = Signal::derive(move || TopicSort::parse(&query.read().get("sort").unwrap_or_default()));
    let category = Resource::new(id, |id| async move {
        match id {
            Some(id) => forum_category(id).await,
            None => Err(crate::error::AppError::NotFound("Kategorie nicht gefunden.".into())),
        }
    });
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match category.await {
                    Err(e) if e.is_not_found() => view! { <Missing text="Diese Kategorie gibt es nicht."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(c) => {
                        let cid = c.id;
                        let post_role = c.post_role;
                        let can_post = move || session.user.with(|u| u.as_ref().is_some_and(|u| u.role.at_least(post_role)));
                        view! {
                            <Title text=c.name.clone()/>
                            <div class="breadcrumbs"><a href="/forum">"Forum"</a><Icon name="chevron-right" size=14/><span>{c.name.clone()}</span></div>
                            <div class="cat-header" style=format!("--cat-color:{}", c.color)>
                                <div>
                                    <h1>{c.name.clone()}</h1>
                                    {c.description.clone().map(|d| view! { <p class="muted">{d}</p> })}
                                </div>
                                <Show when=can_post>
                                    <a class="btn btn-primary" href=format!("/forum/new?category={cid}")><Icon name="plus" size=16/>"Neues Thema"</a>
                                </Show>
                            </div>
                            {move || sort_tabs(format!("/forum/c/{cid}"), sort.get())}
                            <TopicList filter=TopicFilter::Category(cid) sort=sort/>
                        }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}

#[component]
pub fn TagPage() -> impl IntoView {
    let params = use_params_map();
    let query = use_query_map();
    let tag = move || params.read().get("tag").unwrap_or_default();
    let sort = Signal::derive(move || TopicSort::parse(&query.read().get("sort").unwrap_or_default()));
    let filter = Signal::derive(move || TopicFilter::Tag(tag()));
    view! {
        <Title text=move || format!("#{}", tag())/>
        <div class="breadcrumbs"><a href="/forum">"Forum"</a><Icon name="chevron-right" size=14/><span>"Tag"</span></div>
        <h1>"#"{tag}</h1>
        {move || sort_tabs(format!("/forum/tag/{}", tag()), sort.get())}
        <TopicList filter=filter sort=sort/>
    }
}

#[component]
pub fn NewTopicPage() -> impl IntoView {
    view! {
        <Title text="Neues Thema"/>
        <RequireLogin>
            <NewTopicForm/>
        </RequireLogin>
    }
}

#[component]
fn NewTopicForm() -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let query = use_query_map();
    let categories = Resource::new(|| (), |_| forum_categories());
    let category = RwSignal::new(query.with_untracked(|q| q.get("category")).unwrap_or_default());
    let title = RwSignal::new(String::new());
    let tags = RwSignal::new(String::new());
    let body = RwSignal::new(String::new());
    let busy = RwSignal::new(false);

    let submit = Callback::new(move |_| {
        let Ok(category_id) = Uuid::parse_str(&category.get_untracked()) else {
            toasts.error("Bitte wähle eine Kategorie.");
            return;
        };
        busy.set(true);
        let input = NewTopic {
            category_id,
            title: title.get_untracked(),
            body_md: body.get_untracked(),
            tags: tags.get_untracked().split(',').map(|t| t.to_string()).collect(),
        };
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match create_topic(input).await {
                Ok(id) => navigate(&format!("/forum/t/{id}"), Default::default()),
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    });

    view! {
        <PageHeader title="Neues Thema"/>
        <form class="card stack" on:submit=move |ev| { ev.prevent_default(); submit.run(()); }>
            <Suspense fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    let cats = categories.await.unwrap_or_default();
                    let role = session.user.with_untracked(|u| u.as_ref().map(|u| u.role)).unwrap_or_default();
                    let allowed: Vec<CategoryView> = cats.into_iter().filter(|c| role.at_least(c.post_role)).collect();
                    if category.get_untracked().is_empty() {
                        if let Some(first) = allowed.first() {
                            category.set(first.id.to_string());
                        }
                    }
                    view! {
                        <label class="field">
                            <span>"Kategorie"</span>
                            <select bind:value=category required>
                                {allowed.into_iter().map(|c| view! {
                                    <option value=c.id.to_string()>{c.name.clone()}</option>
                                }).collect_view()}
                            </select>
                        </label>
                    }
                })}
            </Suspense>
            <label class="field">
                <span>"Titel"</span>
                <input type="text" required minlength="3" maxlength="200" bind:value=title placeholder="Worum geht es?"/>
            </label>
            <label class="field">
                <span>"Tags (durch Komma getrennt, max. 5)"</span>
                <input type="text" bind:value=tags placeholder="z. B. hilfe, rust"/>
            </label>
            <div class="field">
                <span>"Text"</span>
                <MarkdownEditor value=body rows=12 on_submit=submit placeholder="Schreib deinen Beitrag …"/>
            </div>
            <div class="row end gap">
                <a class="btn" href="/forum">"Abbrechen"</a>
                <button class="btn btn-primary" disabled=move || busy.get()><Icon name="send" size=16/>"Thema erstellen"</button>
            </div>
        </form>
    }
}
