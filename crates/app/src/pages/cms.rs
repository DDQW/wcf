use std::collections::HashMap;

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use serde_json::{Map, Value};
use shared::cms::{
    CmsTypeView, DisplayValue, EntryEdit, EntryQuery, EntryStatus, EntryView, FieldDef, FieldKind,
    ListLayout, RelationRef, ENTRIES_PER_PAGE,
};
use shared::SubjectType;
use uuid::Uuid;

use crate::api::cms::*;
use crate::components::comments::CommentsSection;
use crate::components::common::{confirm, datetime_local_value, parse_datetime_local, sleep_ms};
use crate::components::{
    EmptyState, ErrorBox, Icon, MarkdownEditor, PageHeader, Pager, RequireLogin, Spinner, TimeAgo,
};
use crate::pages::not_found::Missing;
use crate::state::use_toasts;

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

#[component]
fn ValueView(v: DisplayValue) -> impl IntoView {
    match v {
        DisplayValue::Empty => view! { <span class="muted">"–"</span> }.into_any(),
        DisplayValue::Text(t) => view! { <span class="cms-text">{t}</span> }.into_any(),
        DisplayValue::Html(h) => view! { <div class="prose" inner_html=h></div> }.into_any(),
        DisplayValue::Bool(b) => view! { <span class=if b { "badge badge-ok" } else { "badge" }>{if b { "Ja" } else { "Nein" }}</span> }.into_any(),
        DisplayValue::Link { url, label } => view! { <a href=url target="_blank" rel="noopener noreferrer nofollow">{label}</a> }.into_any(),
        DisplayValue::Color(c) => view! { <span class="cms-color"><span class="cat-swatch small-swatch" style=format!("background:{c}")></span>{c.clone()}</span> }.into_any(),
        DisplayValue::Image { url, .. } => {
            let href = url.clone();
            view! { <a href=href target="_blank" rel="external"><img class="cms-image" src=url alt=""/></a> }.into_any()
        }
        DisplayValue::File { url, name } => view! { <a class="btn btn-sm" href=url rel="external" download=""><Icon name="download" size=14/>{name}</a> }.into_any(),
        DisplayValue::Tags(tags) => view! { <span class="tag-row inline">{tags.into_iter().map(|t| view! { <span class="badge">{t}</span> }).collect_view()}</span> }.into_any(),
        DisplayValue::Relations(list) => view! {
            <span class="tag-row inline">{list.into_iter().map(|r| view! { <a class="cat-chip" href=r.url>{r.title}</a> }).collect_view()}</span>
        }.into_any(),
    }
}

fn status_badge(s: EntryStatus) -> impl IntoView {
    (s != EntryStatus::Published).then(|| {
        let class = match s {
            EntryStatus::Scheduled => "badge badge-new",
            EntryStatus::Archived => "badge badge-muted",
            _ => "badge",
        };
        view! { <span class=class>{s.label()}</span> }
    })
}

// ---------------------------------------------------------------------------
// Public list
// ---------------------------------------------------------------------------

#[component]
pub fn CmsListPage() -> impl IntoView {
    let params = use_params_map();
    let slug = move || params.read().get("type").unwrap_or_default();
    let ty = Resource::new(slug, cms_type);
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match ty.await {
                    Ok(t) => view! { <CmsList t=t/> }.into_any(),
                    Err(e) if e.is_not_found() => view! { <Missing text="Diesen Bereich gibt es nicht."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn CmsList(t: CmsTypeView) -> impl IntoView {
    let query = use_query_map();
    let search = RwSignal::new(String::new());
    let debounced = RwSignal::new(String::new());
    let filters = RwSignal::new(HashMap::<String, String>::new());
    let sort = RwSignal::new(String::new());
    let page = RwSignal::new(0i64);
    let all = query.with_untracked(|q| q.get("all").is_some()) && t.can_create;
    let slug = t.slug.clone();
    Effect::new(move |_| {
        let s = search.get();
        leptos::task::spawn_local(async move {
            sleep_ms(250).await;
            if search.get_untracked() == s {
                debounced.set(s);
                page.set(0);
            }
        });
    });
    let slug2 = slug.clone();
    let entries = Resource::new(
        move || (debounced.get(), filters.get(), sort.get(), page.get()),
        move |(q, f, s, p)| {
            let slug = slug2.clone();
            async move {
                let (sort_key, desc) = match s.as_str() {
                    "" => (None, true),
                    "title" => (Some("title".to_string()), false),
                    other => match other.strip_prefix('-') {
                        Some(k) => (Some(k.to_string()), true),
                        None => (Some(other.to_string()), false),
                    },
                };
                cms_entries(slug, EntryQuery {
                    search: (!q.is_empty()).then_some(q),
                    filters: f.into_iter().filter(|(_, v)| !v.is_empty()).collect(),
                    sort: sort_key,
                    descending: desc,
                    page: p,
                    all_statuses: all,
                }).await
            }
        },
    );
    let total = Signal::derive(move || entries.get().and_then(|r| r.ok()).map(|p| p.total).unwrap_or(0));
    let filter_fields: Vec<FieldDef> = t.fields.iter().filter(|f| f.kind.filterable()).cloned().collect();
    let sort_fields: Vec<FieldDef> = t.fields.iter().filter(|f| matches!(f.kind, FieldKind::Number | FieldKind::Integer | FieldKind::Date | FieldKind::Text | FieldKind::Select)).cloned().collect();
    let layout = t.list_layout;
    let list_fields: Vec<FieldDef> = t.fields.iter().filter(|f| f.show_in_list).cloned().collect();

    view! {
        <Title text=t.name_plural.clone()/>
        <PageHeader title=t.name_plural.clone() subtitle=t.description.clone()>
            {t.can_create.then(|| view! {
                {(!all).then(|| view! { <a class="btn btn-ghost" href=format!("/c/{}?all=1", slug)><Icon name="list" size=16/>"Alle inkl. Entwürfe"</a> })}
                <a class="btn btn-primary" href=format!("/cms/{}/new", slug)><Icon name="plus" size=16/>{format!("{} anlegen", t.name)}</a>
            })}
        </PageHeader>
        <div class="toolbar card">
            <input type="search" placeholder="Suchen …" bind:value=search/>
            {filter_fields.into_iter().map(|f| {
                let key = f.key.clone();
                view! {
                    <select on:change=move |ev| { let v = event_target_value(&ev); let k = key.clone(); filters.update(|m| { m.insert(k, v); }); page.set(0); }>
                        <option value="">{format!("{}: alle", f.label)}</option>
                        {if f.kind == FieldKind::Boolean {
                            view! { <option value="true">"Ja"</option><option value="false">"Nein"</option> }.into_any()
                        } else {
                            f.options.iter().map(|o| view! { <option value=o.clone()>{o.clone()}</option> }).collect_view().into_any()
                        }}
                    </select>
                }
            }).collect_view()}
            <select on:change=move |ev| { sort.set(event_target_value(&ev)); page.set(0); }>
                <option value="">"Neueste zuerst"</option>
                <option value="title">"Titel A–Z"</option>
                {sort_fields.into_iter().map(|f| view! {
                    <option value=f.key.clone()>{format!("{} ↑", f.label)}</option>
                    <option value=format!("-{}", f.key)>{format!("{} ↓", f.label)}</option>
                }).collect_view()}
            </select>
        </div>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || {
                let list_fields = list_fields.clone();
                Suspend::new(async move {
                    match entries.await {
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        Ok(p) if p.items.is_empty() => view! { <EmptyState icon="layout" title="Keine Einträge gefunden."/> }.into_any(),
                        Ok(p) => match layout {
                            ListLayout::Table => view! {
                                <div class="card flush table-wrap">
                                    <table class="table">
                                        <thead><tr>
                                            <th>"Titel"</th>
                                            {list_fields.iter().map(|f| view! { <th>{f.label.clone()}</th> }).collect_view()}
                                            <th>"Datum"</th>
                                        </tr></thead>
                                        <tbody>
                                            {p.items.into_iter().map(|e| {
                                                let vals: Vec<_> = e.values.iter().filter(|v| v.show_in_list).cloned().collect();
                                                view! {
                                                    <tr>
                                                        <td><a href=e.url()><strong>{e.title.clone()}</strong></a>" "{status_badge(e.status)}</td>
                                                        {vals.into_iter().map(|v| view! { <td><ValueView v=v.value/></td> }).collect_view()}
                                                        <td class="small muted nowrap">{e.published_at.map(|t| view! { <TimeAgo ts=t/> })}</td>
                                                    </tr>
                                                }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>
                            }.into_any(),
                            ListLayout::List => view! {
                                <div class="card flush">
                                    {p.items.into_iter().map(|e| view! { <EntryRow e=e/> }).collect_view()}
                                </div>
                            }.into_any(),
                            ListLayout::Cards => view! {
                                <div class="cms-grid">
                                    {p.items.into_iter().map(|e| view! { <EntryCard e=e/> }).collect_view()}
                                </div>
                            }.into_any(),
                        },
                    }
                })
            }}
        </Transition>
        <Pager page=page total=total per_page=ENTRIES_PER_PAGE/>
    }
}

#[component]
fn EntryCard(e: EntryView) -> impl IntoView {
    let url = e.url();
    let vals: Vec<_> = e.values.iter().filter(|v| v.show_in_list).cloned().collect();
    view! {
        <article class="cms-card card flush">
            {e.cover_url.clone().map(|c| view! { <a href=url.clone() class="post-card-cover"><img src=c alt="" loading="lazy"/></a> })}
            <div class="post-card-body">
                <a class="post-card-title" href=url.clone()>{e.title.clone()}</a>
                {status_badge(e.status)}
                <dl class="cms-mini">
                    {vals.into_iter().map(|v| view! { <dt>{v.label.clone()}</dt><dd><ValueView v=v.value/></dd> }).collect_view()}
                </dl>
            </div>
        </article>
    }
}

#[component]
fn EntryRow(e: EntryView) -> impl IntoView {
    let vals: Vec<_> = e.values.iter().filter(|v| v.show_in_list).cloned().collect();
    view! {
        <div class="cms-row">
            <div class="grow">
                <a href=e.url()><strong>{e.title.clone()}</strong></a>" "{status_badge(e.status)}
                <div class="cms-row-values">
                    {vals.into_iter().map(|v| view! { <span><span class="muted small">{v.label.clone()}": "</span><ValueView v=v.value/></span> }).collect_view()}
                </div>
            </div>
            <span class="small muted">{e.published_at.map(|t| view! { <TimeAgo ts=t/> })}</span>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Entry & page detail
// ---------------------------------------------------------------------------

#[component]
pub fn CmsEntryPage() -> impl IntoView {
    let params = use_params_map();
    let key = move || (params.read().get("type").unwrap_or_default(), params.read().get("slug").unwrap_or_default());
    let entry = Resource::new(key, |(t, s)| cms_entry(t, s));
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match entry.await {
                    Ok(e) => view! { <EntryDetail e=e page=false/> }.into_any(),
                    Err(e) if e.is_not_found() => view! { <Missing text="Diesen Eintrag gibt es nicht."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
pub fn CmsStaticPage() -> impl IntoView {
    let params = use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();
    let entry = Resource::new(slug, cms_page);
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match entry.await {
                    Ok(e) => view! { <EntryDetail e=e page=true/> }.into_any(),
                    Err(e) if e.is_not_found() => view! { <Missing text="Diese Seite gibt es nicht."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn EntryDetail(e: EntryView, page: bool) -> impl IntoView {
    let id = e.id;
    let (prose, fields): (Vec<_>, Vec<_>) = e.values.iter().cloned().partition(|v| matches!(v.value, DisplayValue::Html(_)));
    let fields: Vec<_> = fields.into_iter().filter(|v| v.value != DisplayValue::Empty).collect();
    view! {
        <Title text=e.title.clone()/>
        <article class="cms-entry">
            {(!page).then(|| view! {
                <div class="breadcrumbs"><a href=format!("/c/{}", e.type_slug)>{e.type_name.clone()}</a></div>
            })}
            <div class="cms-entry-head">
                <h1>{e.title.clone()}" "{status_badge(e.status)}</h1>
                {e.can_edit.then(|| view! { <a class="btn btn-sm" href=format!("/cms/edit/{id}")><Icon name="edit" size=14/>"Bearbeiten"</a> })}
            </div>
            {(!page).then(|| view! {
                <div class="muted small cms-meta">
                    {e.author.clone().map(|a| view! { <a href=a.profile_url()>{a.display_name.clone()}</a>" · " })}
                    {e.published_at.map(|t| view! { <TimeAgo ts=t absolute=true/> })}
                </div>
            })}
            {(!fields.is_empty()).then(|| view! {
                <dl class="cms-fields card">
                    {fields.into_iter().map(|v| view! { <dt>{v.label.clone()}</dt><dd><ValueView v=v.value/></dd> }).collect_view()}
                </dl>
            })}
            {prose.into_iter().map(|v| view! {
                <section class="cms-prose">
                    {(!page).then(|| view! { <h2>{v.label.clone()}</h2> })}
                    <ValueView v=v.value/>
                </section>
            }).collect_view()}
        </article>
        {(e.allow_comments && e.status == EntryStatus::Published).then(|| view! {
            <CommentsSection subject_type=SubjectType::CmsEntry subject_id=id allowed=true/>
        })}
    }
}

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

#[component]
pub fn CmsEditorPage() -> impl IntoView {
    view! {
        <Title text="Inhalt bearbeiten"/>
        <RequireLogin>
            <CmsEditorLoader/>
        </RequireLogin>
    }
}

#[component]
fn CmsEditorLoader() -> impl IntoView {
    let params = use_params_map();
    let id = params.with_untracked(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok()));
    let type_slug = params.with_untracked(|p| p.get("type"));
    let data = Resource::new(
        move || (id, type_slug.clone()),
        |(id, slug)| async move {
            let edit = match id {
                Some(id) => cms_entry_for_edit(id).await?,
                None => EntryEdit {
                    id: None,
                    type_slug: slug.clone().unwrap_or_default(),
                    title: String::new(),
                    slug: String::new(),
                    data: Map::new(),
                    status: EntryStatus::Draft,
                    publish_at: None,
                },
            };
            let ty = cms_type(edit.type_slug.clone()).await?;
            Ok::<_, crate::error::AppError>((edit, ty))
        },
    );
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Ok((edit, ty)) => view! { <EntryForm edit=edit ty=ty/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn EntryForm(edit: EntryEdit, ty: CmsTypeView) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let id = edit.id;
    let title = RwSignal::new(edit.title.clone());
    let slug = RwSignal::new(edit.slug.clone());
    let data = RwSignal::new(edit.data.clone());
    let status = RwSignal::new(match edit.status {
        EntryStatus::Draft => "draft",
        EntryStatus::Published => "published",
        EntryStatus::Scheduled => "scheduled",
        EntryStatus::Archived => "archived",
    }.to_string());
    let when = RwSignal::new(edit.publish_at.map(datetime_local_value).unwrap_or_default());
    let saving = RwSignal::new(false);
    // Markdown editors need plain string signals.
    let md_signals: HashMap<String, RwSignal<String>> = ty
        .fields
        .iter()
        .filter(|f| f.kind == FieldKind::Markdown)
        .map(|f| (f.key.clone(), RwSignal::new(edit.data.get(&f.key).and_then(|v| v.as_str()).unwrap_or_default().to_string())))
        .collect();
    let md_for_save = md_signals.clone();
    let type_slug = ty.slug.clone();

    let save = Callback::new(move |_| {
        let mut d = data.get_untracked();
        for (k, sig) in &md_for_save {
            d.insert(k.clone(), Value::String(sig.get_untracked()));
        }
        let st = match status.get_untracked().as_str() {
            "published" => EntryStatus::Published,
            "scheduled" => EntryStatus::Scheduled,
            "archived" => EntryStatus::Archived,
            _ => EntryStatus::Draft,
        };
        let publish_at = if st == EntryStatus::Scheduled {
            match parse_datetime_local(&when.get_untracked()) {
                Some(t) => Some(t),
                None => {
                    toasts.error("Bitte Datum und Uhrzeit wählen.");
                    return;
                }
            }
        } else {
            parse_datetime_local(&when.get_untracked())
        };
        let input = EntryEdit {
            id,
            type_slug: type_slug.clone(),
            title: title.get_untracked(),
            slug: slug.get_untracked(),
            data: d,
            status: st,
            publish_at,
        };
        saving.set(true);
        leptos::task::spawn_local(async move {
            match save_cms_entry(input).await {
                Ok(url) => {
                    toasts.success("Gespeichert.");
                    go.run(url);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            saving.set(false);
        });
    });
    let remove = move |_| {
        let Some(id) = id else { return };
        if !confirm("Eintrag endgültig löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_cms_entry(id).await {
                Ok(slug) => go.run(format!("/c/{slug}")),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    view! {
        <PageHeader title=if id.is_some() { format!("{} bearbeiten", ty.name) } else { format!("{} anlegen", ty.name) }>
            <a class="btn btn-ghost" href=format!("/c/{}?all=1", ty.slug)>"Zur Liste"</a>
        </PageHeader>
        <div class="editor-layout">
            <div class="stack">
                <section class="card stack">
                    <label class="field"><span>"Titel *"</span><input type="text" class="title-input" maxlength="300" bind:value=title/></label>
                    {ty.fields.iter().cloned().map(|f| {
                        let md = md_signals.get(&f.key).copied();
                        view! { <FieldInput f=f data=data md=md/> }
                    }).collect_view()}
                </section>
            </div>
            <aside class="stack">
                <section class="card stack-sm">
                    <h3>"Status"</h3>
                    <select bind:value=status>
                        <option value="draft">"Entwurf"</option>
                        <option value="published">"Veröffentlicht"</option>
                        <option value="scheduled">"Geplant"</option>
                        <option value="archived">"Archiviert"</option>
                    </select>
                    <Show when=move || status.get() == "scheduled">
                        <input type="datetime-local" bind:value=when/>
                    </Show>
                    <label class="field"><span>"URL-Kürzel"</span><input type="text" bind:value=slug placeholder="automatisch"/></label>
                    <button class="btn btn-primary btn-block" disabled=move || saving.get() on:click=move |_| save.run(())><Icon name="check" size=16/>"Speichern"</button>
                    {id.map(|_| view! { <button class="btn btn-danger-soft btn-block" on:click=remove><Icon name="trash" size=16/>"Löschen"</button> })}
                </section>
                {id.map(|id| view! { <Revisions id=id/> })}
            </aside>
        </div>
    }
}

#[component]
fn Revisions(id: Uuid) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let revs = Resource::new(move || id, cms_revisions);
    view! {
        <section class="card stack-sm">
            <h3><Icon name="clock" size=16/>"Versionen"</h3>
            <Suspense fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    let list = revs.await.unwrap_or_default();
                    if list.is_empty() {
                        return view! { <p class="muted small">"Noch keine früheren Versionen."</p> }.into_any();
                    }
                    view! {
                        <ul class="plain-list rev-list">
                            {list.into_iter().map(|r| {
                                let rid = r.id;
                                view! {
                                    <li>
                                        <div class="grow small">
                                            <TimeAgo ts=r.created_at absolute=true/>
                                            <div class="muted">{r.editor.map(|e| e.display_name).unwrap_or_default()}" · "{r.title.clone()}</div>
                                        </div>
                                        <button class="btn btn-sm" on:click=move |_| {
                                            if !confirm("Diese Version wiederherstellen? Der aktuelle Stand wird als Version gesichert.") { return; }
                                            leptos::task::spawn_local(async move {
                                                match restore_cms_revision(rid).await {
                                                    Ok(url) => { toasts.success("Version wiederhergestellt."); go.run(url); }
                                                    Err(e) => toasts.error(e.to_string()),
                                                }
                                            });
                                        }>"Wiederherstellen"</button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                })}
            </Suspense>
        </section>
    }
}

fn get_str(data: &Map<String, Value>, key: &str) -> String {
    match data.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

#[component]
fn FieldInput(f: FieldDef, data: RwSignal<Map<String, Value>>, md: Option<RwSignal<String>>) -> impl IntoView {
    let toasts = use_toasts();
    let key = f.key.clone();
    let label = format!("{}{}", f.label, if f.required { " *" } else { "" });
    let help = f.help.clone();
    let set = {
        let key = key.clone();
        move |v: Value| data.update(|d| { d.insert(key.clone(), v); })
    };
    let k1 = key.clone();
    let value_str = move || data.with(|d| get_str(d, &k1));
    let input: AnyView = match f.kind {
        FieldKind::Text | FieldKind::Url | FieldKind::Email => {
            let t = match f.kind { FieldKind::Url => "url", FieldKind::Email => "email", _ => "text" };
            let set = set.clone();
            view! { <input type=t prop:value=value_str on:input=move |ev| set(Value::String(event_target_value(&ev)))/> }.into_any()
        }
        FieldKind::LongText => {
            let set = set.clone();
            view! { <textarea rows=4 prop:value=value_str on:input=move |ev| set(Value::String(event_target_value(&ev)))></textarea> }.into_any()
        }
        FieldKind::Markdown => match md {
            Some(sig) => view! { <MarkdownEditor value=sig rows=10/> }.into_any(),
            None => ().into_any(),
        },
        FieldKind::Number | FieldKind::Integer => {
            let set = set.clone();
            let step = if f.kind == FieldKind::Integer { "1" } else { "any" };
            view! {
                <input type="number" step=step min=f.min.map(|m| m.to_string()) max=f.max.map(|m| m.to_string())
                    prop:value=value_str
                    on:input=move |ev| { let s = event_target_value(&ev); set(if s.is_empty() { Value::Null } else { Value::String(s) }); }/>
            }.into_any()
        }
        FieldKind::Boolean => {
            let set = set.clone();
            let k = key.clone();
            view! {
                <label class="check"><input type="checkbox" prop:checked=move || data.with(|d| d.get(&k).and_then(|v| v.as_bool()).unwrap_or(false))
                    on:change=move |ev| set(Value::Bool(event_target_checked(&ev)))/>"Ja"</label>
            }.into_any()
        }
        FieldKind::Date => {
            let set = set.clone();
            view! { <input type="date" prop:value=value_str on:input=move |ev| set(Value::String(event_target_value(&ev)))/> }.into_any()
        }
        FieldKind::DateTime => {
            let set = set.clone();
            let k = key.clone();
            let local = move || data.with(|d| d.get(&k).and_then(|v| v.as_str()).and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|t| datetime_local_value(t.with_timezone(&chrono::Utc))).unwrap_or_default());
            view! {
                <input type="datetime-local" prop:value=local
                    on:input=move |ev| set(parse_datetime_local(&event_target_value(&ev)).map(|t| Value::String(t.to_rfc3339())).unwrap_or(Value::Null))/>
            }.into_any()
        }
        FieldKind::Select => {
            let set = set.clone();
            view! {
                <select prop:value=value_str on:change=move |ev| { let v = event_target_value(&ev); set(if v.is_empty() { Value::Null } else { Value::String(v) }); }>
                    <option value="">"– bitte wählen –"</option>
                    {f.options.iter().map(|o| view! { <option value=o.clone()>{o.clone()}</option> }).collect_view()}
                </select>
            }.into_any()
        }
        FieldKind::MultiSelect => {
            let k = key.clone();
            view! {
                <div class="check-grid">
                    {f.options.iter().map(|o| {
                        let k = k.clone();
                        let k2 = k.clone();
                        let o2 = o.clone();
                        view! {
                            <label class="check">
                                <input type="checkbox"
                                    prop:checked=move || data.with(|d| d.get(&k).and_then(|v| v.as_array()).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(&o2))))
                                    on:change={let o = o.clone(); move |ev| {
                                        let on = event_target_checked(&ev);
                                        data.update(|d| {
                                            let mut list: Vec<Value> = d.get(&k2).and_then(|v| v.as_array().cloned()).unwrap_or_default();
                                            list.retain(|x| x.as_str() != Some(&o));
                                            if on { list.push(Value::String(o.clone())); }
                                            d.insert(k2.clone(), Value::Array(list));
                                        });
                                    }}/>
                                {o.clone()}
                            </label>
                        }
                    }).collect_view()}
                </div>
            }.into_any()
        }
        FieldKind::Color => {
            let set = set.clone();
            let set2 = set.clone();
            let k = key.clone();
            let color = move || data.with(|d| d.get(&k).and_then(|v| v.as_str()).unwrap_or("#000000").to_string());
            view! {
                <div class="row gap">
                    <input type="color" prop:value=color on:input=move |ev| set(Value::String(event_target_value(&ev)))/>
                    <input type="text" placeholder="#rrggbb" prop:value=value_str on:input=move |ev| set2(Value::String(event_target_value(&ev)))/>
                </div>
            }.into_any()
        }
        FieldKind::Image | FieldKind::File => {
            let k = key.clone();
            let is_image = f.kind == FieldKind::Image;
            let uploading = RwSignal::new(false);
            let current = Memo::new(move |_| data.with(|d| d.get(&k).and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok())));
            let set = set.clone();
            let set_clear = set.clone();
            let on_pick = move |ev: leptos::ev::Event| {
                #[cfg(feature = "hydrate")]
                {
                    let files = crate::upload::files_from_input(&ev);
                    let Some(file) = files.into_iter().next() else { return };
                    uploading.set(true);
                    let set = set.clone();
                    leptos::task::spawn_local(async move {
                        let opts = crate::upload::UploadOpts { purpose: shared::files::FilePurpose::Cms, ..Default::default() };
                        match crate::upload::upload_file(file, opts, |_| {}).await {
                            Ok(res) => match (res.files.into_iter().next(), res.errors.into_iter().next()) {
                                (Some(f), _) => set(Value::String(f.id.to_string())),
                                (None, Some(e)) => toasts.error(e),
                                _ => {}
                            },
                            Err(e) => toasts.error(e),
                        }
                        uploading.set(false);
                    });
                }
                #[cfg(not(feature = "hydrate"))]
                let _ = (ev, &set, uploading, toasts);
            };
            view! {
                <div class="row gap wrap">
                    {move || current.get().map(|id| if is_image {
                        view! { <img class="cms-thumb" src=format!("/files/{id}/thumb") alt=""/> }.into_any()
                    } else {
                        view! { <a href=format!("/files/{id}") target="_blank" rel="external"><Icon name="file" size=16/>"Datei ansehen"</a> }.into_any()
                    })}
                    <label class="btn btn-sm"><Icon name="upload" size=14/>{move || if uploading.get() { "Lade …" } else if is_image { "Bild wählen" } else { "Datei wählen" }}
                        <input type="file" accept=if is_image { "image/*" } else { "*/*" } class="hidden" on:change=on_pick/></label>
                    <Show when=move || current.get().is_some()>
                        <button type="button" class="btn btn-sm btn-ghost" on:click={let set_clear = set_clear.clone(); move |_| set_clear(Value::Null)}>"Entfernen"</button>
                    </Show>
                </div>
            }.into_any()
        }
        FieldKind::Relation => view! { <RelationInput f=f.clone() data=data/> }.into_any(),
    };
    view! {
        <div class="field cms-field">
            <span>{label}</span>
            {input}
            {(!help.is_empty()).then(|| view! { <small class="muted">{help.clone()}</small> })}
        </div>
    }
}

#[component]
fn RelationInput(f: FieldDef, data: RwSignal<Map<String, Value>>) -> impl IntoView {
    let key = f.key.clone();
    let target = f.relation_type.clone().unwrap_or_default();
    let multiple = f.multiple;
    let selected = RwSignal::new(Vec::<RelationRef>::new());
    let query = RwSignal::new(String::new());
    let options = RwSignal::new(Vec::<RelationRef>::new());
    let initial: Vec<Uuid> = data.with_untracked(|d| match d.get(&key) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect(),
        Some(Value::String(s)) => Uuid::parse_str(s).ok().into_iter().collect(),
        _ => Vec::new(),
    });
    if !initial.is_empty() {
        leptos::task::spawn_local(async move {
            if let Ok(list) = cms_relation_titles(initial).await {
                selected.set(list);
            }
        });
    }
    let sync = {
        let key = key.clone();
        move || {
            let ids: Vec<Value> = selected.get_untracked().iter().map(|r| Value::String(r.id.to_string())).collect();
            data.update(|d| {
                if multiple {
                    d.insert(key.clone(), Value::Array(ids));
                } else {
                    d.insert(key.clone(), ids.into_iter().next().unwrap_or(Value::Null));
                }
            });
        }
    };
    let t2 = target.clone();
    Effect::new(move |_| {
        let q = query.get();
        let t = t2.clone();
        leptos::task::spawn_local(async move {
            sleep_ms(200).await;
            if query.get_untracked() != q {
                return;
            }
            if let Ok(list) = cms_relation_options(t, q).await {
                options.set(list);
            }
        });
    });
    let sync2 = sync.clone();
    let available = move || {
        let chosen: Vec<Uuid> = selected.with(|s| s.iter().map(|x| x.id).collect());
        options.get().into_iter().filter(|o| !chosen.contains(&o.id)).take(8).collect::<Vec<_>>()
    };
    view! {
        <div class="relation-input">
            <div class="chips">
                <For each=move || selected.get() key=|r| r.id children=move |r| {
                    let id = r.id;
                    let sync = sync2.clone();
                    view! {
                        <span class="chip">{r.title.clone()}
                            <button type="button" on:click=move |_| { selected.update(|v| v.retain(|x| x.id != id)); sync(); }><Icon name="x" size=12/></button>
                        </span>
                    }
                }/>
            </div>
            <input type="search" placeholder=format!("{} suchen …", target) bind:value=query/>
            <ul class="relation-options">
                <For each=available key=|r| r.id children=move |r| {
                    let pick = r.clone();
                    let sync = sync.clone();
                    view! {
                        <li><button type="button" class="link-btn" on:click=move |_| {
                            selected.update(|v| { if !multiple { v.clear(); } v.push(pick.clone()); });
                            sync();
                        }><Icon name="plus" size=12/>{r.title.clone()}</button></li>
                    }
                }/>
            </ul>
        </div>
    }
}

/// Overview of all content types (`/c`).
#[component]
pub fn CmsIndex() -> impl IntoView {
    let types = Resource::new(|| (), |_| cms_types());
    view! {
        <Title text="Inhalte"/>
        <PageHeader title="Inhalte"/>
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match types.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(list) => {
                        let list: Vec<CmsTypeView> = list.into_iter().filter(|t| !t.is_page_type).collect();
                        if list.is_empty() {
                            return view! { <EmptyState icon="layout" title="Noch keine Inhalte."/> }.into_any();
                        }
                        view! {
                            <div class="cms-grid">
                                {list.into_iter().map(|t| view! {
                                    <a class="card cms-type-card" href=format!("/c/{}", t.slug)>
                                        <Icon name=t.icon.clone() size=28/>
                                        <strong>{t.name_plural.clone()}</strong>
                                        <span class="muted small">{t.description.clone()}</span>
                                        <span class="muted small">{format!("{} Einträge", t.entry_count)}</span>
                                    </a>
                                }).collect_view()}
                            </div>
                        }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}
