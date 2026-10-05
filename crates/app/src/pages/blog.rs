use chrono::{DateTime, Utc};
use leptos::prelude::*;
use leptos_meta::{Meta, Title};
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use shared::blog::{BlogFilter, BlogPostEdit, BlogPostItem, BlogView, PostStatus};
#[cfg(feature = "hydrate")]
use shared::files::{FilePurpose, Visibility};
use shared::SubjectType;
use uuid::Uuid;

use crate::api::blog::*;
use crate::components::comments::CommentsSection;
use crate::components::common::{confirm, copy_to_clipboard, origin};
use crate::components::{
    Avatar, EmptyState, ErrorBox, Icon, InfiniteSentinel, MarkdownEditor, PageHeader, ReactionBar,
    ReportButton, RequireLogin, Spinner, TimeAgo,
};
use crate::pages::not_found::Missing;
use crate::state::{use_session, use_toasts};

// ---------------------------------------------------------------------------
// Shared pieces
// ---------------------------------------------------------------------------

#[component]
pub fn PostCard(p: BlogPostItem, #[prop(optional)] show_status: bool) -> impl IntoView {
    let url = p.url();
    view! {
        <article class="post-card card flush">
            <a class="post-card-cover" href=url.clone()>
                {match p.cover_url.clone() {
                    Some(c) => view! { <img src=c alt="" loading="lazy"/> }.into_any(),
                    None => view! { <span class="cover-fallback"><Icon name="book-open" size=36/></span> }.into_any(),
                }}
            </a>
            <div class="post-card-body">
                {show_status.then(|| view! {
                    <span class=match p.status { PostStatus::Published => "badge badge-ok", PostStatus::Scheduled => "badge badge-new", PostStatus::Draft => "badge" }>
                        {p.status.label()}
                    </span>
                })}
                <a class="post-card-title" href=url.clone()>{p.title.clone()}</a>
                <p class="post-card-excerpt">{p.excerpt.clone()}</p>
                <div class="post-card-meta">
                    <Avatar user=p.author.clone() size=22/>
                    <a href=format!("/blog/{}", p.author.username)>{p.author.display_name.clone()}</a>
                    <span class="muted">"·"</span>
                    {p.published_at.map(|t| view! { <TimeAgo ts=t/> })}
                    <span class="grow"></span>
                    <span class="muted small" title="Lesezeit"><Icon name="clock" size=13/>{format!("{} Min.", p.reading_minutes)}</span>
                    <span class="muted small" title="Kommentare"><Icon name="message-circle" size=13/>{p.comment_count}</span>
                    <span class="muted small" title="Reaktionen"><Icon name="heart" size=13/>{p.like_count}</span>
                </div>
            </div>
        </article>
    }
}

#[component]
fn PostGrid(#[prop(into)] filter: Signal<BlogFilter>, #[prop(optional, into)] empty: String) -> impl IntoView {
    let first = Resource::new(move || filter.get(), |f| list_blog_posts(f, None));
    let extra = RwSignal::new(Vec::<BlogPostItem>::new());
    let cursor = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    Effect::new(move |_| {
        filter.track();
        extra.set(Vec::new());
    });
    let more = move |_| {
        let Some(c) = cursor.get_untracked() else { return };
        loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(page) = list_blog_posts(filter.get_untracked(), Some(c)).await {
                extra.update(|v| v.extend(page.items));
                cursor.set(page.next_cursor);
            }
            loading.set(false);
        });
    };
    let empty = if empty.is_empty() { "Noch keine Beiträge.".to_string() } else { empty };
    view! {
        <Transition fallback=|| view! { <Spinner/> }>
            {move || {
                let empty = empty.clone();
                Suspend::new(async move {
                    match first.await {
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        Ok(page) if page.items.is_empty() => view! { <EmptyState icon="book-open" title=empty/> }.into_any(),
                        Ok(page) => {
                            cursor.set(page.next_cursor.clone());
                            view! {
                                <div class="post-grid">
                                    {page.items.into_iter().map(|p| view! { <PostCard p=p/> }).collect_view()}
                                    <For each=move || extra.get() key=|p| p.id children=|p| view! { <PostCard p=p/> }/>
                                </div>
                                <InfiniteSentinel on_visible=more enabled=Signal::derive(move || cursor.with(|c| c.is_some())) loading=loading/>
                            }.into_any()
                        }
                    }
                })
            }}
        </Transition>
    }
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

#[component]
pub fn BlogsIndex() -> impl IntoView {
    let session = use_session();
    let query = use_query_map();
    let tab = move || query.read().get("view").unwrap_or_default();
    let tags = Resource::new(|| (), |_| blog_tags());
    view! {
        <Title text="Blogs"/>
        <PageHeader title="Blogs" subtitle="Beiträge aus der Community">
            <a class="btn btn-ghost" href="/blogs/feed.xml" rel="external" title="RSS"><Icon name="rss" size=16/>"RSS"</a>
            <Show when=move || session.logged_in()>
                <a class="btn" href="/me/blog"><Icon name="book-open" size=16/>"Mein Blog"</a>
                <a class="btn btn-primary" href="/me/blog/new"><Icon name="edit" size=16/>"Schreiben"</a>
            </Show>
        </PageHeader>
        <div class="tabs">
            <a class="tab" class:active=move || tab().is_empty() href="/blogs">"Neueste Beiträge"</a>
            <a class="tab" class:active=move || tab() == "blogs" href="/blogs?view=blogs">"Alle Blogs"</a>
            <Show when=move || session.logged_in()>
                <a class="tab" class:active=move || tab() == "following" href="/blogs?view=following">"Abonniert"</a>
            </Show>
        </div>
        <div class="forum-layout">
            <div>
                {move || match tab().as_str() {
                    "blogs" => view! { <BlogList/> }.into_any(),
                    "following" => view! { <PostGrid filter=BlogFilter::Following empty="Du hast noch keine Blogs abonniert."/> }.into_any(),
                    _ => view! { <PostGrid filter=BlogFilter::All/> }.into_any(),
                }}
            </div>
            <aside class="card">
                <h3><Icon name="tag" size=16/>"Themen"</h3>
                <Suspense fallback=|| ()>
                    {move || Suspend::new(async move {
                        let list = tags.await.unwrap_or_default();
                        if list.is_empty() {
                            return view! { <p class="muted small">"Noch keine Tags."</p> }.into_any();
                        }
                        view! {
                            <div class="tag-cloud">
                                {list.into_iter().map(|t| view! {
                                    <a class="tag-chip" href=format!("/blogs/tag/{}", t.tag)>"#"{t.tag.clone()}<span class="muted small">" "{t.count}</span></a>
                                }).collect_view()}
                            </div>
                        }.into_any()
                    })}
                </Suspense>
            </aside>
        </div>
    }
}

#[component]
fn BlogList() -> impl IntoView {
    let blogs = Resource::new(|| (), |_| all_blogs());
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match blogs.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(list) if list.is_empty() => view! { <EmptyState icon="book-open" title="Noch keine Blogs."/> }.into_any(),
                    Ok(list) => view! {
                        <div class="blog-list">
                            {list.into_iter().map(|b| view! {
                                <a class="blog-card card" href=b.url()>
                                    <Avatar user=b.owner.clone() size=48/>
                                    <div class="grow">
                                        <strong>{b.title.clone()}</strong>
                                        <p class="muted small">{b.description.clone()}</p>
                                        <div class="muted small">
                                            {b.owner.display_name.clone()} " · " {b.post_count} " Beiträge · " {b.subscriber_count} " Abonnenten"
                                        </div>
                                    </div>
                                </a>
                            }).collect_view()}
                        </div>
                    }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
pub fn BlogTagPage() -> impl IntoView {
    let params = use_params_map();
    let tag = move || params.read().get("tag").unwrap_or_default();
    let filter = Signal::derive(move || BlogFilter::Tag(tag()));
    view! {
        <Title text=move || format!("#{} – Blogs", tag())/>
        <div class="breadcrumbs"><a href="/blogs">"Blogs"</a><Icon name="chevron-right" size=14/><span>"Tag"</span></div>
        <h1>"#"{tag}</h1>
        <PostGrid filter=filter/>
    }
}

#[component]
pub fn BlogPage() -> impl IntoView {
    let params = use_params_map();
    let username = move || params.read().get("username").unwrap_or_default();
    let blog = Resource::new(username, get_blog);
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match blog.await {
                    Err(e) if e.is_not_found() => view! { <Missing text="Diesen Blog gibt es (noch) nicht."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(b) => view! { <BlogHeader b=b.clone()/><PostGrid filter=BlogFilter::Blog(b.id) empty="In diesem Blog gibt es noch keine Beiträge."/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn BlogHeader(b: BlogView) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let subscribed = RwSignal::new(b.subscribed);
    let subs = RwSignal::new(b.subscriber_count);
    let blog_id = b.id;
    let owner_id = b.owner.id;
    let toggle = move |_| {
        let on = !subscribed.get_untracked();
        leptos::task::spawn_local(async move {
            match subscribe_blog(blog_id, on).await {
                Ok(()) => {
                    subscribed.set(on);
                    subs.update(|n| *n += if on { 1 } else { -1 });
                    toasts.success(if on { "Blog abonniert – du wirst über neue Beiträge benachrichtigt." } else { "Abo beendet." });
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Title text=b.title.clone()/>
        <section class="blog-header card">
            <Avatar user=b.owner.clone() size=72/>
            <div class="grow">
                <h1>{b.title.clone()}</h1>
                {(!b.description.is_empty()).then(|| view! { <p>{b.description.clone()}</p> })}
                <div class="muted small">
                    "von " <a href=b.owner.profile_url()>{b.owner.display_name.clone()}</a>
                    " · " {b.post_count} " Beiträge · " {move || subs.get()} " Abonnenten"
                </div>
            </div>
            <div class="row gap">
                <a class="btn btn-ghost" href=format!("/blog/{}/feed.xml", b.owner.username) rel="external" title="RSS"><Icon name="rss" size=16/></a>
                {move || if session.user_id() == Some(owner_id) {
                    view! {
                        <a class="btn" href="/me/blog"><Icon name="sliders" size=16/>"Verwalten"</a>
                        <a class="btn btn-primary" href="/me/blog/new"><Icon name="edit" size=16/>"Neuer Beitrag"</a>
                    }.into_any()
                } else if session.logged_in() {
                    view! {
                        <button class="btn" class:btn-primary=move || !subscribed.get() on:click=toggle>
                            <Icon name="bell" size=16/>{move || if subscribed.get() { "Abonniert" } else { "Abonnieren" }}
                        </button>
                    }.into_any()
                } else {
                    ().into_any()
                }}
            </div>
        </section>
    }
}

#[component]
pub fn BlogPostPage() -> impl IntoView {
    let params = use_params_map();
    let key = move || (params.read().get("username").unwrap_or_default(), params.read().get("slug").unwrap_or_default());
    let post = Resource::new(key, |(u, s)| get_blog_post(u, s));
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match post.await {
                    Err(e) if e.is_not_found() => view! { <Missing text="Diesen Beitrag gibt es nicht (mehr)."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(d) => {
                        let p = d.post.clone();
                        let id = p.id;
                        let blog_url = d.blog.url();
                        let remove = move |_| {
                            if !confirm("Diesen Beitrag endgültig löschen?") { return; }
                            let blog_url = blog_url.clone();
                            leptos::task::spawn_local(async move {
                                match delete_blog_post(id).await {
                                    Ok(()) => go.run(blog_url),
                                    Err(e) => toasts.error(e.to_string()),
                                }
                            });
                        };
                        let share_url = p.url();
                        view! {
                            <Title text=p.title.clone()/>
                            <Meta name="description" content=p.excerpt.clone()/>
                            <article class="blog-post">
                                {p.cover_url.clone().map(|c| view! { <img class="blog-cover" src=c alt=""/> })}
                                <div class="breadcrumbs">
                                    <a href="/blogs">"Blogs"</a><Icon name="chevron-right" size=14/>
                                    <a href=d.blog.url()>{d.blog.title.clone()}</a>
                                </div>
                                {(p.status != PostStatus::Published).then(|| view! {
                                    <div class="alert"><Icon name="eye-off" size=16/>
                                        {format!("{} – nur für dich sichtbar.", p.status.label())}
                                        {p.published_at.filter(|_| p.status == PostStatus::Scheduled).map(|t| view! { " Erscheint " <TimeAgo ts=t absolute=true/> })}
                                    </div>
                                })}
                                <h1 class="blog-title">{p.title.clone()}</h1>
                                <div class="blog-byline">
                                    <Avatar user=p.author.clone() size=36/>
                                    <div>
                                        <a class="user-link" href=p.author.profile_url()>{p.author.display_name.clone()}</a>
                                        <div class="muted small">
                                            {p.published_at.map(|t| view! { <TimeAgo ts=t absolute=true/> })}
                                            " · " {format!("{} Min. Lesezeit", p.reading_minutes)} " · " {p.view_count} " Aufrufe"
                                        </div>
                                    </div>
                                    <span class="grow"></span>
                                    {d.can_edit.then(|| view! {
                                        <a class="btn btn-sm" href=format!("/me/blog/edit/{id}")><Icon name="edit" size=14/>"Bearbeiten"</a>
                                        <button class="btn btn-sm btn-danger-soft" on:click=remove.clone()><Icon name="trash" size=14/></button>
                                    })}
                                </div>
                                <div class="tag-row">{p.tags.iter().map(|t| view! { <a class="tag-chip" href=format!("/blogs/tag/{t}")>"#"{t.clone()}</a> }).collect_view()}</div>
                                <div class="prose blog-body" inner_html=d.body_html.clone()></div>
                                <footer class="blog-foot">
                                    <ReactionBar subject_type=SubjectType::BlogPost subject_id=id reactions=d.reactions.clone()/>
                                    <span class="grow"></span>
                                    <button class="btn btn-sm btn-ghost" on:click=move |_| { copy_to_clipboard(&format!("{}{}", origin(), share_url)); toasts.success("Link kopiert."); }>
                                        <Icon name="share" size=14/>"Teilen"
                                    </button>
                                    <ReportButton subject_type=SubjectType::BlogPost subject_id=id compact=true/>
                                </footer>
                                <nav class="post-nav">
                                    {d.older.clone().map(|n| view! { <a class="card" href=n.url><span class="muted small">"← Älter"</span><strong>{n.title}</strong></a> })}
                                    <span class="grow"></span>
                                    {d.newer.clone().map(|n| view! { <a class="card align-right" href=n.url><span class="muted small">"Neuer →"</span><strong>{n.title}</strong></a> })}
                                </nav>
                            </article>
                            {(p.status == PostStatus::Published).then(|| view! {
                                <CommentsSection subject_type=SubjectType::BlogPost subject_id=id allowed=d.allow_comments/>
                            })}
                        }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}

// ---------------------------------------------------------------------------
// Own blog management
// ---------------------------------------------------------------------------

#[component]
pub fn MyBlogPage() -> impl IntoView {
    view! {
        <Title text="Mein Blog"/>
        <RequireLogin>
            <MyBlog/>
        </RequireLogin>
    }
}

#[component]
fn MyBlog() -> impl IntoView {
    let toasts = use_toasts();
    let blog = Resource::new(|| (), |_| my_blog());
    let posts = Resource::new(|| (), |_| my_blog_posts());
    let title = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        leptos::task::spawn_local(async move {
            match save_my_blog(title.get_untracked(), description.get_untracked()).await {
                Ok(_) => {
                    toasts.success("Blog gespeichert.");
                    blog.refetch();
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <PageHeader title="Mein Blog">
            <Suspense fallback=|| ()>
                {move || Suspend::new(async move {
                    blog.await.ok().flatten().map(|b| view! { <a class="btn" href=b.url()><Icon name="eye" size=16/>"Ansehen"</a> })
                })}
            </Suspense>
            <a class="btn btn-primary" href="/me/blog/new"><Icon name="edit" size=16/>"Neuer Beitrag"</a>
        </PageHeader>
        <div class="settings-grid">
            <Suspense fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    let b = blog.await.ok().flatten();
                    title.set(b.as_ref().map(|b| b.title.clone()).unwrap_or_default());
                    description.set(b.as_ref().map(|b| b.description.clone()).unwrap_or_default());
                    view! {
                        <form class="card stack" on:submit=save>
                            <h2>"Einstellungen"</h2>
                            {b.is_none().then(|| view! { <p class="muted small">"Du hast noch keinen Blog. Lege ihn hier an – oder schreib einfach los."</p> })}
                            <label class="field"><span>"Titel"</span><input type="text" required maxlength="100" bind:value=title/></label>
                            <label class="field"><span>"Beschreibung"</span><textarea rows=3 bind:value=description></textarea></label>
                            <div class="row end"><button class="btn btn-primary">{if b.is_some() { "Speichern" } else { "Blog anlegen" }}</button></div>
                        </form>
                    }
                })}
            </Suspense>
            <section class="card">
                <h2>"Beiträge"</h2>
                <Suspense fallback=|| view! { <Spinner/> }>
                    {move || Suspend::new(async move {
                        match posts.await {
                            Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                            Ok(list) if list.is_empty() => view! { <p class="muted">"Noch keine Beiträge."</p> }.into_any(),
                            Ok(list) => view! {
                                <ul class="my-posts">
                                    {list.into_iter().map(|p| view! {
                                        <li>
                                            <div class="grow">
                                                <a href=p.url()><strong>{p.title.clone()}</strong></a>
                                                <div class="muted small">
                                                    {p.status.label()} " · "
                                                    {match p.published_at { Some(t) => view! { <TimeAgo ts=t absolute=true/> }.into_any(), None => view! { <TimeAgo ts=p.updated_at/> }.into_any() }}
                                                    " · " {p.view_count} " Aufrufe · " {p.comment_count} " Kommentare"
                                                </div>
                                            </div>
                                            <a class="btn btn-sm" href=format!("/me/blog/edit/{}", p.id)><Icon name="edit" size=14/>"Bearbeiten"</a>
                                        </li>
                                    }).collect_view()}
                                </ul>
                            }.into_any(),
                        }
                    })}
                </Suspense>
            </section>
        </div>
    }
}

/// "2026-10-05T14:30" in local time for `<input type=datetime-local>`.
fn to_local_input(ts: DateTime<Utc>) -> String {
    #[cfg(feature = "hydrate")]
    {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts.timestamp_millis() as f64));
        format!(
            "{}-{:02}-{:02}T{:02}:{:02}",
            d.get_full_year(),
            d.get_month() + 1,
            d.get_date(),
            d.get_hours(),
            d.get_minutes()
        )
    }
    #[cfg(not(feature = "hydrate"))]
    {
        ts.format("%Y-%m-%dT%H:%M").to_string()
    }
}

fn from_local_input(s: &str) -> Option<DateTime<Utc>> {
    if s.trim().is_empty() {
        return None;
    }
    #[cfg(feature = "hydrate")]
    {
        let ms = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(s)).get_time();
        if ms.is_nan() {
            return None;
        }
        DateTime::<Utc>::from_timestamp_millis(ms as i64)
    }
    #[cfg(not(feature = "hydrate"))]
    {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").ok().map(|n| n.and_utc())
    }
}

#[component]
pub fn BlogEditorPage() -> impl IntoView {
    view! {
        <Title text="Beitrag schreiben"/>
        <RequireLogin>
            <BlogEditor/>
        </RequireLogin>
    }
}

#[component]
fn BlogEditor() -> impl IntoView {
    let params = use_params_map();
    let id = params.with_untracked(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok()));
    let data = Resource::new(move || id, |id| async move {
        match id {
            Some(id) => blog_post_for_edit(id).await,
            None => Ok(BlogPostEdit { allow_comments: true, ..Default::default() }),
        }
    });
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Ok(p) => view! { <EditorForm initial=p/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn EditorForm(initial: BlogPostEdit) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let id = initial.id;
    let title = RwSignal::new(initial.title.clone());
    let slug = RwSignal::new(initial.slug.clone());
    let excerpt = RwSignal::new(initial.excerpt.clone());
    let body = RwSignal::new(initial.body_md.clone());
    let tags = RwSignal::new(initial.tags.join(", "));
    let cover = RwSignal::new(initial.cover_file_id);
    let cover_url = RwSignal::new(initial.cover_url.clone());
    let allow_comments = RwSignal::new(initial.allow_comments);
    let mode = RwSignal::new(match (initial.publish, initial.publish_at) {
        (true, Some(t)) if t > Utc::now() => "schedule",
        (true, _) => "publish",
        _ => "draft",
    }.to_string());
    let when = RwSignal::new(initial.publish_at.map(to_local_input).unwrap_or_default());
    let saving = RwSignal::new(false);
    let uploading = RwSignal::new(false);

    let on_cover = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            let files = crate::upload::files_from_input(&ev);
            let Some(f) = files.into_iter().next() else { return };
            uploading.set(true);
            leptos::task::spawn_local(async move {
                let opts = crate::upload::UploadOpts { purpose: FilePurpose::Blog, visibility: Some(Visibility::Public), ..Default::default() };
                match crate::upload::upload_file(f, opts, |_| {}).await {
                    Ok(res) => match (res.files.into_iter().next(), res.errors.into_iter().next()) {
                        (Some(file), _) if file.has_thumb => {
                            cover_url.set(Some(file.preview_url()));
                            cover.set(Some(file.id));
                        }
                        (Some(_), _) => toasts.error("Bitte ein Bild hochladen."),
                        (None, Some(e)) => toasts.error(e),
                        _ => {}
                    },
                    Err(e) => toasts.error(e),
                }
                uploading.set(false);
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = (ev, uploading);
    };

    let save = Callback::new(move |_| {
        let m = mode.get_untracked();
        let publish_at = match m.as_str() {
            "schedule" => match from_local_input(&when.get_untracked()) {
                Some(t) => Some(t),
                None => {
                    toasts.error("Bitte Datum und Uhrzeit für die Veröffentlichung wählen.");
                    return;
                }
            },
            "publish" => initial_publish_at(id, &when.get_untracked()),
            _ => None,
        };
        let input = BlogPostEdit {
            id,
            title: title.get_untracked(),
            slug: slug.get_untracked(),
            excerpt: excerpt.get_untracked(),
            body_md: body.get_untracked(),
            cover_file_id: cover.get_untracked(),
            cover_url: None,
            tags: tags.get_untracked().split(',').map(|s| s.to_string()).collect(),
            publish: m != "draft",
            publish_at,
            allow_comments: allow_comments.get_untracked(),
        };
        saving.set(true);
        leptos::task::spawn_local(async move {
            match save_blog_post(input).await {
                Ok(saved) => {
                    toasts.success(match m.as_str() {
                        "draft" => "Entwurf gespeichert.",
                        "schedule" => "Beitrag geplant.",
                        _ => "Beitrag veröffentlicht.",
                    });
                    go.run(saved.url);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            saving.set(false);
        });
    });

    view! {
        <PageHeader title=if id.is_some() { "Beitrag bearbeiten" } else { "Neuer Beitrag" }>
            <a class="btn btn-ghost" href="/me/blog">"Zurück"</a>
        </PageHeader>
        <div class="editor-layout">
            <div class="stack">
                <input type="text" class="title-input" placeholder="Titel" maxlength="200" bind:value=title/>
                <MarkdownEditor value=body rows=22 on_submit=save placeholder="Schreib deinen Beitrag … (Markdown, Bilder per Upload-Knopf)"/>
            </div>
            <aside class="stack">
                <section class="card stack-sm">
                    <h3>"Veröffentlichen"</h3>
                    <label class="radio"><input type="radio" name="mode" checked=move || mode.get() == "draft" on:change=move |_| mode.set("draft".into())/>"Entwurf"</label>
                    <label class="radio"><input type="radio" name="mode" checked=move || mode.get() == "publish" on:change=move |_| mode.set("publish".into())/>"Veröffentlicht"</label>
                    <label class="radio"><input type="radio" name="mode" checked=move || mode.get() == "schedule" on:change=move |_| mode.set("schedule".into())/>"Geplant"</label>
                    <Show when=move || mode.get() == "schedule">
                        <input type="datetime-local" bind:value=when/>
                    </Show>
                    <label class="check"><input type="checkbox" bind:checked=allow_comments/>"Kommentare erlauben"</label>
                    <button class="btn btn-primary btn-block" disabled=move || saving.get() || uploading.get() on:click=move |_| save.run(())>
                        <Icon name="check" size=16/>"Speichern"
                    </button>
                </section>
                <section class="card stack-sm">
                    <h3>"Titelbild"</h3>
                    {move || cover_url.get().map(|u| view! { <img class="cover-preview" src=u alt=""/> })}
                    <div class="row gap">
                        <label class="btn btn-sm"><Icon name="image" size=14/>{move || if uploading.get() { "Lade …" } else { "Bild wählen" }}
                            <input type="file" accept="image/*" class="hidden" on:change=on_cover/></label>
                        <Show when=move || cover.get().is_some()>
                            <button class="btn btn-sm btn-ghost" on:click=move |_| { cover.set(None); cover_url.set(None); }>"Entfernen"</button>
                        </Show>
                    </div>
                </section>
                <section class="card stack-sm">
                    <label class="field"><span>"Tags (Komma-getrennt)"</span><input type="text" bind:value=tags/></label>
                    <label class="field"><span>"URL-Kürzel (optional)"</span><input type="text" bind:value=slug placeholder="wird aus dem Titel erzeugt"/></label>
                    <label class="field"><span>"Auszug (optional)"</span><textarea rows=3 maxlength="500" bind:value=excerpt></textarea></label>
                </section>
                {id.map(|post_id| view! { <DeletePost id=post_id/> })}
            </aside>
        </div>
    }
}

fn initial_publish_at(id: Option<Uuid>, when: &str) -> Option<DateTime<Utc>> {
    // Re-saving an already published post keeps its date.
    id.and(from_local_input(when)).filter(|t| *t <= Utc::now())
}

#[component]
fn DeletePost(id: Uuid) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let remove = move |_| {
        if !confirm("Diesen Beitrag endgültig löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_blog_post(id).await {
                Ok(()) => go.run("/me/blog".into()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! { <button class="btn btn-danger-soft" on:click=remove><Icon name="trash" size=16/>"Beitrag löschen"</button> }
}
