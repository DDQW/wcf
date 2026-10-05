use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use shared::cms::{key_from_label, CmsTypeInput, CmsTypeView, FieldDef, FieldKind, ListLayout, MenuItem};
use shared::forum::CategoryAccess;
use shared::Role;
use uuid::Uuid;

use crate::api::cms::{admin_cms_types, cms_menu, delete_cms_type, save_cms_menu, save_cms_type};
use crate::components::common::confirm;
use crate::components::{EmptyState, ErrorBox, Icon, Spinner};
use crate::state::{use_session, use_toasts};

const ICONS: [&str; 14] = [
    "layout", "file-text", "calendar", "map-pin", "star", "film", "music", "book-open", "tag",
    "users", "globe", "image", "zap", "archive",
];

#[component]
pub fn AdminCms() -> impl IntoView {
    let types = Resource::new(|| (), |_| admin_cms_types());
    view! {
        <div class="page-header">
            <h1>"Inhaltstypen"</h1>
            <div class="page-actions">
                <a class="btn" href="/admin/menu"><Icon name="menu" size=16/>"Menü"</a>
                <a class="btn btn-primary" href="/admin/cms/type/new"><Icon name="plus" size=16/>"Neuer Typ"</a>
            </div>
        </div>
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match types.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(list) if list.is_empty() => view! { <EmptyState icon="layout" title="Noch keine Inhaltstypen."/> }.into_any(),
                    Ok(list) => view! {
                        <div class="card flush">
                            <ul class="cat-admin-list">
                                {list.into_iter().map(|t| view! {
                                    <li class="cat-admin-row">
                                        <Icon name=t.icon.clone() size=20/>
                                        <div class="grow">
                                            <strong>{t.name_plural.clone()}</strong>
                                            {t.is_page_type.then(|| view! { <span class="badge">"eingebaut"</span> })}
                                            <div class="muted small">
                                                {format!("/{} · {} Felder · {} Einträge · ", if t.is_page_type { "p".to_string() } else { format!("c/{}", t.slug) }, t.fields.len(), t.entry_count)}
                                                {t.read_access.label()}
                                            </div>
                                        </div>
                                        <a class="btn btn-sm" href=format!("/c/{}?all=1", t.slug)><Icon name="list" size=14/>"Einträge"</a>
                                        <a class="btn btn-sm" href=format!("/cms/{}/new", t.slug)><Icon name="plus" size=14/></a>
                                        <a class="btn btn-sm" href=format!("/admin/cms/type/{}", t.id)><Icon name="edit" size=14/>"Felder"</a>
                                    </li>
                                }).collect_view()}
                            </ul>
                        </div>
                    }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
pub fn AdminCmsType() -> impl IntoView {
    let params = use_params_map();
    let id = params.with_untracked(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok()));
    let types = Resource::new(|| (), |_| admin_cms_types());
    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match types.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(list) => {
                        let current = id.and_then(|id| list.iter().find(|t| t.id == id).cloned());
                        let slugs: Vec<String> = list.iter().map(|t| t.slug.clone()).collect();
                        view! { <TypeEditor current=current all_slugs=slugs/> }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}

#[component]
fn TypeEditor(current: Option<CmsTypeView>, all_slugs: Vec<String>) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let id = current.as_ref().map(|t| t.id);
    let is_page = current.as_ref().is_some_and(|t| t.is_page_type);
    let name = RwSignal::new(current.as_ref().map(|t| t.name.clone()).unwrap_or_default());
    let plural = RwSignal::new(current.as_ref().map(|t| t.name_plural.clone()).unwrap_or_default());
    let slug = RwSignal::new(current.as_ref().map(|t| t.slug.clone()).unwrap_or_default());
    let description = RwSignal::new(current.as_ref().map(|t| t.description.clone()).unwrap_or_default());
    let icon = RwSignal::new(current.as_ref().map(|t| t.icon.clone()).unwrap_or_else(|| "layout".into()));
    let access = RwSignal::new(current.as_ref().map(|t| t.read_access.as_str()).unwrap_or("public").to_string());
    let create_role = RwSignal::new(current.as_ref().map(|t| t.create_role.as_str()).unwrap_or("moderator").to_string());
    let comments = RwSignal::new(current.as_ref().is_some_and(|t| t.allow_comments));
    let layout = RwSignal::new(current.as_ref().map(|t| t.list_layout.as_str()).unwrap_or("cards").to_string());
    let fields = RwSignal::new(current.as_ref().map(|t| t.fields.clone()).unwrap_or_default());

    Effect::new(move |_| {
        // Suggest a slug for new types.
        if id.is_none() {
            let n = name.get();
            slug.set(shared::util::slug_base(&n));
        }
    });

    let add_field = move |_| {
        fields.update(|f| {
            let n = f.len() + 1;
            f.push(FieldDef { key: format!("feld_{n}"), label: format!("Feld {n}"), ..Default::default() });
        });
    };
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = CmsTypeInput {
            slug: slug.get_untracked(),
            name: name.get_untracked(),
            name_plural: plural.get_untracked(),
            description: description.get_untracked(),
            icon: icon.get_untracked(),
            fields: fields.get_untracked(),
            read_access: CategoryAccess::parse(&access.get_untracked()),
            create_role: Role::parse(&create_role.get_untracked()).unwrap_or(Role::Moderator),
            allow_comments: comments.get_untracked(),
            list_layout: ListLayout::parse(&layout.get_untracked()),
        };
        leptos::task::spawn_local(async move {
            match save_cms_type(id, input).await {
                Ok(_) => {
                    toasts.success("Inhaltstyp gespeichert.");
                    go.run("/admin/cms".into());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let remove = move |_| {
        let Some(id) = id else { return };
        if !confirm("Inhaltstyp mit allen Einträgen löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_cms_type(id).await {
                Ok(()) => go.run("/admin/cms".into()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let slugs = StoredValue::new(all_slugs);
    let field_indices = move || (0..fields.with(|f| f.len())).collect::<Vec<_>>();
    let field_key = move |i: &usize| (*i, fields.with(|f| f.len()));

    view! {
        <h1>{if id.is_some() { "Inhaltstyp bearbeiten" } else { "Neuer Inhaltstyp" }}</h1>
        <form class="stack" on:submit=save>
            <section class="card stack">
                <div class="row gap wrap">
                    <label class="field grow"><span>"Name (Einzahl)"</span><input type="text" required placeholder="z. B. Veranstaltung" bind:value=name/></label>
                    <label class="field grow"><span>"Name (Mehrzahl)"</span><input type="text" required placeholder="z. B. Veranstaltungen" bind:value=plural/></label>
                    <label class="field"><span>"Kürzel (URL)"</span><input type="text" required disabled=is_page bind:value=slug/></label>
                </div>
                <label class="field"><span>"Beschreibung"</span><input type="text" bind:value=description/></label>
                <div class="row gap wrap">
                    <label class="field"><span>"Symbol"</span>
                        <select bind:value=icon>{ICONS.iter().map(|i| view! { <option value=*i>{*i}</option> }).collect_view()}</select>
                    </label>
                    <label class="field"><span>"Wer darf lesen?"</span>
                        <select bind:value=access>{CategoryAccess::ALL.into_iter().map(|a| view! { <option value=a.as_str()>{a.label()}</option> }).collect_view()}</select>
                    </label>
                    <label class="field"><span>"Wer darf anlegen?"</span>
                        <select bind:value=create_role>
                            <option value="member">"Alle Mitglieder"</option>
                            <option value="moderator">"Moderatoren"</option>
                            <option value="admin">"Admins"</option>
                        </select>
                    </label>
                    <label class="field"><span>"Darstellung"</span>
                        <select bind:value=layout>{[ListLayout::Cards, ListLayout::List, ListLayout::Table].into_iter().map(|l| view! { <option value=l.as_str()>{l.label()}</option> }).collect_view()}</select>
                    </label>
                </div>
                <label class="check"><input type="checkbox" bind:checked=comments/>"Kommentare erlauben"</label>
            </section>

            <section class="card stack">
                <div class="row gap">
                    <h2 class="grow">"Felder"</h2>
                    <button type="button" class="btn btn-sm" on:click=add_field><Icon name="plus" size=14/>"Feld hinzufügen"</button>
                </div>
                <p class="muted small">"Jeder Eintrag hat automatisch einen Titel. Hier legst du die zusätzlichen Felder fest."</p>
                <Show when=move || fields.with(|f| f.is_empty())>
                    <p class="muted">"Noch keine Felder."</p>
                </Show>
                <For each=field_indices key=field_key children=move |i| view! { <FieldRow index=i fields=fields slugs=slugs/> }/>
            </section>

            <div class="row gap">
                {(id.is_some() && !is_page).then(|| view! { <button type="button" class="btn btn-danger-soft" on:click=remove><Icon name="trash" size=16/>"Typ löschen"</button> })}
                <span class="grow"></span>
                <a class="btn" href="/admin/cms">"Abbrechen"</a>
                <button class="btn btn-primary"><Icon name="check" size=16/>"Speichern"</button>
            </div>
        </form>
    }
}

#[component]
fn FieldRow(index: usize, fields: RwSignal<Vec<FieldDef>>, slugs: StoredValue<Vec<String>>) -> impl IntoView {
    let get = move || fields.with(|f| f.get(index).cloned()).unwrap_or_default();
    let update = move |fun: &dyn Fn(&mut FieldDef)| {
        fields.update(|f| {
            if let Some(field) = f.get_mut(index) {
                fun(field);
            }
        })
    };
    let options_text = move || get().options.join("\n");
    let move_by = move |delta: isize| {
        fields.update(|f| {
            let j = index as isize + delta;
            if j >= 0 && (j as usize) < f.len() {
                f.swap(index, j as usize);
            }
        })
    };
    view! {
        <div class="field-row">
            <div class="row gap wrap">
                <label class="field grow"><span>"Beschriftung"</span>
                    <input type="text" prop:value=move || get().label on:input=move |ev| {
                        let v = event_target_value(&ev);
                        update(&|f: &mut FieldDef| {
                            let auto = f.key.starts_with("feld_") || f.key == key_from_label(&f.label);
                            f.label = v.clone();
                            if auto { f.key = key_from_label(&v); }
                        });
                    }/>
                </label>
                <label class="field"><span>"Feldname"</span>
                    <input type="text" class="mono" prop:value=move || get().key on:input=move |ev| { let v = event_target_value(&ev); update(&|f: &mut FieldDef| f.key = v.clone()); }/>
                </label>
                <label class="field"><span>"Typ"</span>
                    <select prop:value=move || get().kind.as_str().to_string() on:change=move |ev| { let k = FieldKind::parse(&event_target_value(&ev)); update(&|f: &mut FieldDef| f.kind = k); }>
                        {FieldKind::ALL.into_iter().map(|k| view! { <option value=k.as_str()>{k.label()}</option> }).collect_view()}
                    </select>
                </label>
                <div class="field-row-tools">
                    <button type="button" class="icon-btn" title="Nach oben" on:click=move |_| move_by(-1)><Icon name="arrow-up" size=15/></button>
                    <button type="button" class="icon-btn" title="Nach unten" on:click=move |_| move_by(1)><Icon name="arrow-down" size=15/></button>
                    <button type="button" class="icon-btn" title="Entfernen" on:click=move |_| fields.update(|f| { if index < f.len() { f.remove(index); } })><Icon name="trash" size=15/></button>
                </div>
            </div>
            <div class="row gap wrap">
                <label class="check"><input type="checkbox" prop:checked=move || get().required on:change=move |ev| { let c = event_target_checked(&ev); update(&|f: &mut FieldDef| f.required = c); }/>"Pflichtfeld"</label>
                <label class="check"><input type="checkbox" prop:checked=move || get().show_in_list on:change=move |ev| { let c = event_target_checked(&ev); update(&|f: &mut FieldDef| f.show_in_list = c); }/>"In Listen zeigen"</label>
                <input type="text" class="grow" placeholder="Hilfetext (optional)" prop:value=move || get().help on:input=move |ev| { let v = event_target_value(&ev); update(&|f: &mut FieldDef| f.help = v.clone()); }/>
            </div>
            <Show when=move || get().kind.has_options()>
                <label class="field"><span>"Optionen (eine pro Zeile)"</span>
                    <textarea rows=3 prop:value=options_text on:input=move |ev| {
                        let v = event_target_value(&ev);
                        update(&|f: &mut FieldDef| f.options = v.lines().map(|l| l.to_string()).collect());
                    }></textarea>
                </label>
            </Show>
            <Show when=move || get().kind.has_range()>
                <div class="row gap">
                    <label class="field"><span>{move || if matches!(get().kind, FieldKind::Text | FieldKind::LongText) { "Min. Länge" } else { "Minimum" }}</span>
                        <input type="number" step="any" prop:value=move || get().min.map(|m| m.to_string()).unwrap_or_default()
                            on:input=move |ev| { let v = event_target_value(&ev).parse::<f64>().ok(); update(&|f: &mut FieldDef| f.min = v); }/>
                    </label>
                    <label class="field"><span>{move || if matches!(get().kind, FieldKind::Text | FieldKind::LongText) { "Max. Länge" } else { "Maximum" }}</span>
                        <input type="number" step="any" prop:value=move || get().max.map(|m| m.to_string()).unwrap_or_default()
                            on:input=move |ev| { let v = event_target_value(&ev).parse::<f64>().ok(); update(&|f: &mut FieldDef| f.max = v); }/>
                    </label>
                </div>
            </Show>
            <Show when=move || get().kind == FieldKind::Relation>
                <div class="row gap">
                    <label class="field"><span>"Verknüpft mit"</span>
                        <select prop:value=move || get().relation_type.unwrap_or_default() on:change=move |ev| { let v = event_target_value(&ev); update(&|f: &mut FieldDef| f.relation_type = Some(v.clone())); }>
                            <option value="">"– Typ wählen –"</option>
                            {slugs.get_value().into_iter().map(|s| view! { <option value=s.clone()>{s.clone()}</option> }).collect_view()}
                        </select>
                    </label>
                    <label class="check"><input type="checkbox" prop:checked=move || get().multiple on:change=move |ev| { let c = event_target_checked(&ev); update(&|f: &mut FieldDef| f.multiple = c); }/>"Mehrere erlaubt"</label>
                </div>
            </Show>
        </div>
    }
}

#[component]
pub fn AdminMenu() -> impl IntoView {
    let toasts = use_toasts();
    let session = use_session();
    let items = RwSignal::new(Vec::<MenuItem>::new());
    let loaded = RwSignal::new(false);
    Effect::new(move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(list) = cms_menu().await {
                items.set(list);
            }
            loaded.set(true);
        });
    });
    let item_indices = move || (0..items.with(|v| v.len())).collect::<Vec<_>>();
    let item_key = move |i: &usize| (*i, items.with(|v| v.len()));
    let add = move |_| items.update(|v| v.push(MenuItem { id: Uuid::now_v7(), label: String::new(), url: "/".into(), position: v.len() as i32 }));
    let save = move |_| {
        let list = items.get_untracked();
        leptos::task::spawn_local(async move {
            match save_cms_menu(list).await {
                Ok(()) => {
                    toasts.success("Menü gespeichert.");
                    if let Ok(b) = crate::api::auth::load_bootstrap().await {
                        session.apply(b);
                    }
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <h1>"Navigationsmenü"</h1>
        <p class="muted">"Zusätzliche Einträge in der Seitenleiste, z. B. für CMS-Seiten (/p/impressum) oder Inhaltstypen (/c/veranstaltungen)."</p>
        <div class="card stack">
            <Show when=move || loaded.get() fallback=|| view! { <Spinner/> }>
                <For each=item_indices key=item_key children=move |i| {
                    let get = move || items.with(|v| v.get(i).cloned());
                    view! {
                        <div class="row gap menu-row">
                            <input type="text" placeholder="Beschriftung" prop:value=move || get().map(|m| m.label).unwrap_or_default()
                                on:input=move |ev| { let v = event_target_value(&ev); items.update(|l| if let Some(m) = l.get_mut(i) { m.label = v; }); }/>
                            <input type="text" placeholder="/p/seite" prop:value=move || get().map(|m| m.url).unwrap_or_default()
                                on:input=move |ev| { let v = event_target_value(&ev); items.update(|l| if let Some(m) = l.get_mut(i) { m.url = v; }); }/>
                            <button class="icon-btn" on:click=move |_| items.update(|l| if i > 0 { l.swap(i, i - 1); })><Icon name="arrow-up" size=15/></button>
                            <button class="icon-btn" on:click=move |_| items.update(|l| if i + 1 < l.len() { l.swap(i, i + 1); })><Icon name="arrow-down" size=15/></button>
                            <button class="icon-btn" on:click=move |_| items.update(|l| { if i < l.len() { l.remove(i); } })><Icon name="trash" size=15/></button>
                        </div>
                    }
                }/>
                <div class="row gap">
                    <button class="btn" on:click=add><Icon name="plus" size=16/>"Eintrag"</button>
                    <span class="grow"></span>
                    <button class="btn btn-primary" on:click=save><Icon name="check" size=16/>"Speichern"</button>
                </div>
            </Show>
        </div>
    }
}
