use leptos::prelude::*;
use shared::forum::{CategoryAccess, CategoryInput, CategoryView, CATEGORY_COLORS};
use shared::Role;
use uuid::Uuid;

use crate::api::forum::{delete_category, forum_categories, save_category};
use crate::components::{EmptyState, ErrorBox, Icon, Modal, Spinner};
use crate::state::use_toasts;

#[component]
pub fn AdminForum() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let data = Resource::new(move || version.get(), |_| forum_categories());
    let editing = RwSignal::new(None::<Option<CategoryView>>);
    let refresh = Callback::new(move |_| version.update(|v| *v += 1));
    view! {
        <div class="page-header">
            <h1>"Forum-Kategorien"</h1>
            <button class="btn btn-primary" on:click=move |_| editing.set(Some(None))><Icon name="plus" size=16/>"Neue Kategorie"</button>
        </div>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(cats) if cats.is_empty() => view! { <EmptyState icon="message-square" title="Noch keine Kategorien."/> }.into_any(),
                    Ok(cats) => {
                        let all = cats.clone();
                        let parents: Vec<CategoryView> = cats.iter().filter(|c| c.parent_id.is_none()).cloned().collect();
                        view! {
                            <div class="card flush">
                                <ul class="cat-admin-list">
                                    {parents.into_iter().map(|p| {
                                        let children: Vec<CategoryView> = all.iter().filter(|c| c.parent_id == Some(p.id)).cloned().collect();
                                        view! {
                                            <CatAdminRow c=p.clone() editing=editing/>
                                            {children.into_iter().map(|c| view! { <CatAdminRow c=c editing=editing child=true/> }).collect_view()}
                                        }
                                    }).collect_view()}
                                </ul>
                            </div>
                            {move || editing.get().map(|current| view! {
                                <CategoryEditor current=current all=all.clone() editing=editing on_saved=refresh/>
                            })}
                        }.into_any()
                    }
                }
            })}
        </Transition>
    }
}

#[component]
fn CatAdminRow(c: CategoryView, editing: RwSignal<Option<Option<CategoryView>>>, #[prop(optional)] child: bool) -> impl IntoView {
    let edit = c.clone();
    view! {
        <li class="cat-admin-row" class:child=child>
            <span class="cat-swatch" style=format!("background:{}", c.color)></span>
            <div class="grow">
                <strong>{c.name.clone()}</strong>
                <div class="muted small">
                    {c.read_access.label()} " · Themen erstellen: " {c.post_role.label()} " · "
                    {c.topic_count} " Themen · Position " {c.position}
                </div>
            </div>
            <button class="btn btn-sm" on:click=move |_| editing.set(Some(Some(edit.clone())))><Icon name="edit" size=14/>"Bearbeiten"</button>
        </li>
    }
}

#[component]
fn CategoryEditor(
    current: Option<CategoryView>,
    all: Vec<CategoryView>,
    editing: RwSignal<Option<Option<CategoryView>>>,
    on_saved: Callback<()>,
) -> impl IntoView {
    let toasts = use_toasts();
    let open = RwSignal::new(true);
    Effect::new(move |_| {
        if !open.get() {
            editing.set(None);
        }
    });
    let id = current.as_ref().map(|c| c.id);
    let name = RwSignal::new(current.as_ref().map(|c| c.name.clone()).unwrap_or_default());
    let description = RwSignal::new(current.as_ref().and_then(|c| c.description.clone()).unwrap_or_default());
    let color = RwSignal::new(current.as_ref().map(|c| c.color.clone()).unwrap_or_else(|| CATEGORY_COLORS[0].into()));
    let parent = RwSignal::new(current.as_ref().and_then(|c| c.parent_id).map(|p| p.to_string()).unwrap_or_default());
    let access = RwSignal::new(current.as_ref().map(|c| c.read_access.as_str()).unwrap_or("public").to_string());
    let post_role = RwSignal::new(current.as_ref().map(|c| c.post_role.as_str()).unwrap_or("member").to_string());
    let position = RwSignal::new(current.as_ref().map(|c| c.position).unwrap_or(0).to_string());
    let move_to = RwSignal::new(String::new());

    let parents: Vec<CategoryView> = all.iter().filter(|c| c.parent_id.is_none() && Some(c.id) != id).cloned().collect();
    let others: Vec<CategoryView> = all.iter().filter(|c| Some(c.id) != id).cloned().collect();

    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = CategoryInput {
            name: name.get_untracked(),
            description: description.get_untracked(),
            color: color.get_untracked(),
            parent_id: Uuid::parse_str(&parent.get_untracked()).ok(),
            read_access: CategoryAccess::parse(&access.get_untracked()),
            post_role: Role::parse(&post_role.get_untracked()).unwrap_or_default(),
            position: position.get_untracked().parse().unwrap_or(0),
        };
        leptos::task::spawn_local(async move {
            match save_category(id, input).await {
                Ok(_) => {
                    toasts.success("Kategorie gespeichert.");
                    open.set(false);
                    on_saved.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let remove = move |_| {
        let Some(id) = id else { return };
        if !crate::components::common::confirm("Kategorie wirklich löschen?") {
            return;
        }
        let target = Uuid::parse_str(&move_to.get_untracked()).ok();
        leptos::task::spawn_local(async move {
            match delete_category(id, target).await {
                Ok(()) => {
                    toasts.success("Kategorie gelöscht.");
                    open.set(false);
                    on_saved.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    view! {
        <Modal open=open title=if id.is_some() { "Kategorie bearbeiten" } else { "Neue Kategorie" }>
            <form class="stack" on:submit=save>
                <label class="field"><span>"Name"</span><input type="text" required maxlength="60" bind:value=name/></label>
                <label class="field"><span>"Beschreibung"</span><textarea rows=2 bind:value=description></textarea></label>
                <div class="field">
                    <span>"Farbe"</span>
                    <div class="swatches">
                        {CATEGORY_COLORS.iter().map(|c| {
                            let c = c.to_string();
                            let c2 = c.clone();
                            view! {
                                <button type="button" class="swatch" class:active={let c = c.clone(); move || color.get() == c}
                                    style=format!("background:{c}") on:click=move |_| color.set(c2.clone())></button>
                            }
                        }).collect_view()}
                        <input type="color" bind:value=color/>
                    </div>
                </div>
                <div class="row gap wrap">
                    <label class="field grow">
                        <span>"Oberkategorie"</span>
                        <select bind:value=parent>
                            <option value="">"– keine –"</option>
                            {parents.iter().map(|p| view! { <option value=p.id.to_string()>{p.name.clone()}</option> }).collect_view()}
                        </select>
                    </label>
                    <label class="field">
                        <span>"Position"</span>
                        <input type="number" bind:value=position/>
                    </label>
                </div>
                <div class="row gap wrap">
                    <label class="field grow">
                        <span>"Lesen"</span>
                        <select bind:value=access>
                            {CategoryAccess::ALL.into_iter().map(|a| view! { <option value=a.as_str()>{a.label()}</option> }).collect_view()}
                        </select>
                    </label>
                    <label class="field grow">
                        <span>"Themen erstellen ab"</span>
                        <select bind:value=post_role>
                            <option value="member">"Mitglied"</option>
                            <option value="moderator">"Moderator"</option>
                            <option value="admin">"Admin"</option>
                        </select>
                    </label>
                </div>
                <div class="row end"><button class="btn btn-primary"><Icon name="check" size=16/>"Speichern"</button></div>
                {id.map(|_| view! {
                    <hr/>
                    <div class="stack-sm">
                        <strong>"Löschen"</strong>
                        <div class="row gap wrap">
                            <select bind:value=move_to class="grow">
                                <option value="">"Themen nicht verschieben (nur leere Kategorie)"</option>
                                {others.iter().map(|c| view! { <option value=c.id.to_string()>{format!("Themen verschieben nach: {}", c.name)}</option> }).collect_view()}
                            </select>
                            <button type="button" class="btn btn-danger" on:click=remove><Icon name="trash" size=16/>"Löschen"</button>
                        </div>
                    </div>
                })}
            </form>
        </Modal>
    }
}
