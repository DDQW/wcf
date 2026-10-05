use std::collections::HashSet;

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use shared::files::{
    DriveListing, DriveSort, FileKind, FileView, FolderView, ShareInfo, ShareView, StorageUsage,
    Visibility,
};
use shared::util::format_bytes;
use uuid::Uuid;

use crate::api::files::*;
use crate::components::common::{confirm, copy_to_clipboard, origin, sleep_ms};
use crate::components::uploads::{FileThumb, UploadQueue, UploadQueueView};
use crate::components::{
    Avatar, Dropdown, EmptyState, ErrorBox, Icon, Modal, RequireLogin, Spinner, TimeAgo,
};
use crate::state::use_toasts;
use crate::upload::UploadOpts;

#[component]
pub fn FilesPage() -> impl IntoView {
    view! {
        <Title text="Dateien"/>
        <RequireLogin>
            <Drive/>
        </RequireLogin>
    }
}

#[derive(Clone, PartialEq)]
enum Target {
    File(FileView),
    Folder(FolderView),
}

#[component]
fn Drive() -> impl IntoView {
    let toasts = use_toasts();
    let query = use_query_map();
    let navigate = use_navigate();
    let folder_id = Memo::new(move |_| query.with(|q| q.get("folder").and_then(|s| Uuid::parse_str(&s).ok())));
    let trash = Memo::new(move |_| query.with(|q| q.get("view").as_deref() == Some("trash")));
    let sort = RwSignal::new(DriveSort::Name);
    let grid = RwSignal::new(true);
    let search = RwSignal::new(String::new());
    let debounced = RwSignal::new(String::new());
    let version = RwSignal::new(0u32);
    let selected = RwSignal::new(HashSet::<Uuid>::new());
    let queue = UploadQueue::new();
    let dragging = RwSignal::new(false);

    let preview = RwSignal::new(None::<FileView>);
    let preview_open = RwSignal::new(false);
    let rename_target = RwSignal::new(None::<Target>);
    let move_target = RwSignal::new(None::<Vec<Uuid>>);
    let move_folder_target = RwSignal::new(None::<FolderView>);
    let share_target = RwSignal::new(None::<Target>);
    let new_folder_open = RwSignal::new(false);

    Effect::new(move |_| {
        let s = search.get();
        leptos::task::spawn_local(async move {
            sleep_ms(250).await;
            if search.get_untracked() == s {
                debounced.set(s);
            }
        });
    });
    Effect::new(move |_| {
        folder_id.track();
        trash.track();
        selected.set(HashSet::new());
    });

    let listing = Resource::new(
        move || (folder_id.get(), sort.get(), debounced.get(), trash.get(), version.get()),
        |(folder, sort, q, trash, _)| async move {
            if trash {
                list_trash().await
            } else {
                list_drive(folder, sort, (!q.trim().is_empty()).then_some(q)).await
            }
        },
    );
    let reload = Callback::new(move |_| version.update(|v| *v += 1));

    let on_uploaded = Callback::new(move |_f: FileView| reload.run(()));
    let start_upload = move |files: Vec<web_sys::File>| {
        let opts = UploadOpts {
            folder_id: folder_id.get_untracked(),
            ..Default::default()
        };
        #[cfg(feature = "hydrate")]
        queue.start(files, opts, on_uploaded);
        #[cfg(not(feature = "hydrate"))]
        let _ = (files, opts, on_uploaded);
    };
    let on_pick = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        start_upload(crate::upload::files_from_input(&ev));
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };
    let on_drop = move |ev: leptos::ev::DragEvent| {
        ev.prevent_default();
        dragging.set(false);
        if trash.get_untracked() {
            return;
        }
        #[cfg(feature = "hydrate")]
        start_upload(crate::upload::files_from_drop(&ev));
    };
    #[cfg(not(feature = "hydrate"))]
    let _ = start_upload;

    let bulk = move |action: &'static str| {
        let ids: Vec<Uuid> = selected.get_untracked().into_iter().collect();
        if ids.is_empty() {
            return;
        }
        match action {
            "move" => move_target.set(Some(ids)),
            "trash" | "restore" | "purge" => {
                if action == "purge" && !confirm("Ausgewählte Dateien endgültig löschen?") {
                    return;
                }
                leptos::task::spawn_local(async move {
                    let res = match action {
                        "trash" => trash_files(ids).await,
                        "restore" => restore_files(ids).await,
                        _ => purge_files(ids).await,
                    };
                    match res {
                        Ok(()) => {
                            selected.set(HashSet::new());
                            reload.run(());
                        }
                        Err(e) => toasts.error(e.to_string()),
                    }
                });
            }
            vis => {
                let v = Visibility::parse(vis);
                leptos::task::spawn_local(async move {
                    match set_files_visibility(ids, v).await {
                        Ok(()) => {
                            toasts.success(format!("Sichtbarkeit: {}", v.label()));
                            reload.run(());
                        }
                        Err(e) => toasts.error(e.to_string()),
                    }
                });
            }
        }
    };

    let open_file = Callback::new(move |f: FileView| {
        preview.set(Some(f));
        preview_open.set(true);
    });
    let open_folder = Callback::new({
        let navigate = navigate.clone();
        move |id: Uuid| navigate(&format!("/files?folder={id}"), Default::default())
    });
    let ctx = DriveCtx {
        reload,
        selected,
        open_file,
        open_folder,
        rename_target,
        move_target,
        move_folder_target,
        share_target,
        trash,
    };
    provide_context(ctx);

    let empty_trash_click = move |_| {
        if !confirm("Papierkorb endgültig leeren?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match empty_trash().await {
                Ok(()) => reload.run(()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    view! {
        <div class="drive"
            on:dragover=move |ev: leptos::ev::DragEvent| { ev.prevent_default(); if !trash.get_untracked() { dragging.set(true); } }
            on:dragleave=move |_| dragging.set(false)
            on:drop=on_drop
        >
            <div class="page-header">
                <h1>{move || if trash.get() { "Papierkorb" } else { "Dateien" }}</h1>
                <div class="page-actions">
                    <Show when=move || !trash.get() fallback=move || view! {
                        <button class="btn btn-danger-soft" on:click=empty_trash_click><Icon name="trash" size=16/>"Papierkorb leeren"</button>
                    }>
                        <button class="btn" on:click=move |_| new_folder_open.set(true)><Icon name="folder-plus" size=16/>"Neuer Ordner"</button>
                        <label class="btn btn-primary">
                            <Icon name="upload" size=16/>"Hochladen"
                            <input type="file" multiple class="hidden" on:change=on_pick/>
                        </label>
                    </Show>
                </div>
            </div>
            <div class="tabs">
                <a class="tab" class:active=move || !trash.get() href="/files"><Icon name="hard-drive" size=16/>"Meine Dateien"</a>
                <a class="tab" class:active=move || trash.get() href="/files?view=trash"><Icon name="trash" size=16/>"Papierkorb"</a>
            </div>
            <Show when=move || !trash.get()>
                <div class="toolbar card">
                    <input type="search" placeholder="In meinen Dateien suchen …" bind:value=search/>
                    <select prop:value=move || sort.get().as_str().to_string() on:change=move |ev| sort.set(DriveSort::parse(&event_target_value(&ev)))>
                        {[DriveSort::Name, DriveSort::Newest, DriveSort::Size, DriveSort::Type].into_iter().map(|s| view! {
                            <option value=s.as_str()>{s.label()}</option>
                        }).collect_view()}
                    </select>
                    <div class="seg">
                        <button class:active=move || grid.get() title="Raster" on:click=move |_| grid.set(true)><Icon name="grid" size=16/></button>
                        <button class:active=move || !grid.get() title="Liste" on:click=move |_| grid.set(false)><Icon name="list" size=16/></button>
                    </div>
                </div>
            </Show>
            <UploadQueueView queue=queue/>
            <Show when=move || !selected.with(|s| s.is_empty())>
                <div class="bulk-bar card">
                    <strong>{move || format!("{} ausgewählt", selected.with(|s| s.len()))}</strong>
                    <span class="grow"></span>
                    {move || if trash.get() {
                        view! {
                            <button class="btn btn-sm" on:click=move |_| bulk("restore")><Icon name="rotate-cw" size=14/>"Wiederherstellen"</button>
                            <button class="btn btn-sm btn-danger-soft" on:click=move |_| bulk("purge")><Icon name="trash" size=14/>"Endgültig löschen"</button>
                        }.into_any()
                    } else {
                        view! {
                            <button class="btn btn-sm" on:click=move |_| bulk("move")><Icon name="arrow-right" size=14/>"Verschieben"</button>
                            <select class="select-sm" on:change=move |ev| { let v = event_target_value(&ev); if !v.is_empty() { bulk(match v.as_str() { "public" => "public", "members" => "members", _ => "private" }); } }>
                                <option value="">"Sichtbarkeit …"</option>
                                {Visibility::ALL.into_iter().map(|v| view! { <option value=v.as_str()>{v.label()}</option> }).collect_view()}
                            </select>
                            <button class="btn btn-sm btn-danger-soft" on:click=move |_| bulk("trash")><Icon name="trash" size=14/>"Löschen"</button>
                        }.into_any()
                    }}
                    <button class="btn btn-sm btn-ghost" on:click=move |_| selected.set(HashSet::new())>"Abbrechen"</button>
                </div>
            </Show>
            <Transition fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    match listing.await {
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        Ok(l) => view! { <ListingView l=l grid=grid.get_untracked() grid_sig=grid/> }.into_any(),
                    }
                })}
            </Transition>
            <Show when=move || dragging.get()>
                <div class="drop-overlay"><Icon name="upload" size=48/><p>"Dateien hier ablegen"</p></div>
            </Show>
        </div>

        <NewFolderModal open=new_folder_open parent=Signal::derive(move || folder_id.get())/>
        <PreviewModal open=preview_open file=preview/>
        <RenameModal target=rename_target/>
        <MoveModal ids=move_target folder=move_folder_target/>
        <ShareModal target=share_target/>
    }
}

#[derive(Clone, Copy)]
struct DriveCtx {
    reload: Callback<()>,
    selected: RwSignal<HashSet<Uuid>>,
    open_file: Callback<FileView>,
    open_folder: Callback<Uuid>,
    rename_target: RwSignal<Option<Target>>,
    move_target: RwSignal<Option<Vec<Uuid>>>,
    move_folder_target: RwSignal<Option<FolderView>>,
    share_target: RwSignal<Option<Target>>,
    trash: Memo<bool>,
}

fn usage_bar(u: &StorageUsage) -> impl IntoView {
    let pct = u.quota_bytes.map(|q| (u.used_bytes as f64 / q.max(1) as f64 * 100.0).min(100.0));
    view! {
        <div class="usage">
            <div class="usage-text small muted">
                {format_bytes(u.used_bytes)}
                {u.quota_bytes.map(|q| format!(" von {}", format_bytes(q))).unwrap_or_else(|| " belegt (unbegrenzt)".into())}
                {(u.trash_bytes > 0).then(|| format!(" · {} im Papierkorb", format_bytes(u.trash_bytes)))}
                {format!(" · {} Dateien", u.file_count)}
            </div>
            {pct.map(|p| view! {
                <div class="progress"><div class="progress-bar" class:warn={p > 90.0} style=format!("width:{p:.1}%")></div></div>
            })}
        </div>
    }
}

#[component]
fn ListingView(l: DriveListing, grid: bool, grid_sig: RwSignal<bool>) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let _ = grid;
    let empty = l.folders.is_empty() && l.files.is_empty();
    let usage = l.usage.clone();
    let folders = StoredValue::new(l.folders.clone());
    let files = StoredValue::new(l.files.clone());
    let all_ids: Vec<Uuid> = l.files.iter().map(|f| f.id).collect();
    let all_ids2 = all_ids.clone();
    let all_selected = move || !all_ids.is_empty() && ctx.selected.with(|s| all_ids.iter().all(|id| s.contains(id)));
    let toggle_all = move |_| {
        let ids = all_ids2.clone();
        ctx.selected.update(|s| {
            if ids.iter().all(|id| s.contains(id)) {
                s.clear();
            } else {
                s.extend(ids);
            }
        });
    };
    view! {
        {usage_bar(&usage)}
        <Show when=move || !ctx.trash.get()>
            <nav class="breadcrumbs drive-crumbs">
                <a href="/files"><Icon name="hard-drive" size=15/>"Meine Dateien"</a>
                {l.breadcrumbs.iter().map(|b| view! {
                    <Icon name="chevron-right" size=14/>
                    <a href=format!("/files?folder={}", b.id)>{b.name.clone()}</a>
                }).collect_view()}
            </nav>
        </Show>
        {if empty {
            view! {
                <EmptyState icon=if ctx.trash.get_untracked() { "trash" } else { "folder" }
                    title=if ctx.trash.get_untracked() { "Der Papierkorb ist leer." } else { "Noch keine Dateien hier." }>
                    <Show when=move || !ctx.trash.get()>
                        <p class="muted small">"Ziehe Dateien hierher oder nutze „Hochladen“."</p>
                    </Show>
                </EmptyState>
            }.into_any()
        } else {
            view! {
                <div class="select-all row gap-sm small muted">
                    <input type="checkbox" prop:checked=all_selected on:change=toggle_all/>
                    "Alle auswählen"
                </div>
                {move || if grid_sig.get() {
                    view! {
                        <div class="file-grid">
                            {folders.get_value().into_iter().map(|f| view! { <FolderCard f=f/> }).collect_view()}
                            {files.get_value().into_iter().map(|f| view! { <FileCard f=f/> }).collect_view()}
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="card flush table-wrap">
                            <table class="table file-table">
                                <thead><tr><th></th><th>"Name"</th><th>"Größe"</th><th>"Geändert"</th><th>"Sichtbar"</th><th></th></tr></thead>
                                <tbody>
                                    {folders.get_value().into_iter().map(|f| view! { <FolderRow f=f/> }).collect_view()}
                                    {files.get_value().into_iter().map(|f| view! { <FileRow f=f/> }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                    }.into_any()
                }}
            }.into_any()
        }}
    }
}

fn folder_actions(f: FolderView) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let toasts = use_toasts();
    let id = f.id;
    let run = move |fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::error::AppError>>>>| {
        leptos::task::spawn_local(async move {
            match fut.await {
                Ok(()) => ctx.reload.run(()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let f1 = f.clone();
    let f2 = f.clone();
    let f3 = f.clone();
    view! {
        <Dropdown align_right=true label="Aktionen" trigger=|| view! { <Icon name="more-vertical" size=16/> }>
            {if ctx.trash.get_untracked() {
                view! {
                    <button class="menu-item" on:click=move |_| run(Box::pin(restore_folder(id)))><Icon name="rotate-cw" size=15/>"Wiederherstellen"</button>
                    <button class="menu-item danger" on:click=move |_| { if confirm("Ordner mit Inhalt endgültig löschen?") { run(Box::pin(purge_folder(id))) } }><Icon name="trash" size=15/>"Endgültig löschen"</button>
                }.into_any()
            } else {
                view! {
                    <button class="menu-item" on:click={let f = f1.clone(); move |_| ctx.rename_target.set(Some(Target::Folder(f.clone())))}><Icon name="edit" size=15/>"Umbenennen"</button>
                    <button class="menu-item" on:click={let f = f2.clone(); move |_| ctx.move_folder_target.set(Some(f.clone()))}><Icon name="arrow-right" size=15/>"Verschieben"</button>
                    <button class="menu-item" on:click={let f = f3.clone(); move |_| ctx.share_target.set(Some(Target::Folder(f.clone())))}><Icon name="share" size=15/>"Teilen"</button>
                    <button class="menu-item danger" on:click=move |_| run(Box::pin(trash_folder(id)))><Icon name="trash" size=15/>"Löschen"</button>
                }.into_any()
            }}
        </Dropdown>
    }
}

fn file_actions(f: FileView) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let toasts = use_toasts();
    let id = f.id;
    let run = move |fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::error::AppError>>>>| {
        leptos::task::spawn_local(async move {
            match fut.await {
                Ok(()) => ctx.reload.run(()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let download = f.download_url();
    let (fa, fb, fc) = (f.clone(), f.clone(), f.clone());
    view! {
        <Dropdown align_right=true label="Aktionen" trigger=|| view! { <Icon name="more-vertical" size=16/> }>
            {if ctx.trash.get_untracked() {
                view! {
                    <button class="menu-item" on:click=move |_| run(Box::pin(restore_files(vec![id])))><Icon name="rotate-cw" size=15/>"Wiederherstellen"</button>
                    <button class="menu-item danger" on:click=move |_| { if confirm("Datei endgültig löschen?") { run(Box::pin(purge_files(vec![id]))) } }><Icon name="trash" size=15/>"Endgültig löschen"</button>
                }.into_any()
            } else {
                view! {
                    <button class="menu-item" on:click={let f = fa.clone(); move |_| ctx.open_file.run(f.clone())}><Icon name="eye" size=15/>"Öffnen"</button>
                    <a class="menu-item" href=download.clone() rel="external" download=""><Icon name="download" size=15/>"Herunterladen"</a>
                    <button class="menu-item" on:click={let f = fb.clone(); move |_| ctx.rename_target.set(Some(Target::File(f.clone())))}><Icon name="edit" size=15/>"Umbenennen"</button>
                    <button class="menu-item" on:click=move |_| ctx.move_target.set(Some(vec![id]))><Icon name="arrow-right" size=15/>"Verschieben"</button>
                    <button class="menu-item" on:click={let f = fc.clone(); move |_| ctx.share_target.set(Some(Target::File(f.clone())))}><Icon name="share" size=15/>"Teilen"</button>
                    <button class="menu-item" on:click=move |_| { copy_to_clipboard(&format!("{}/files/{id}", origin())); toasts.success("Link kopiert (nur für Berechtigte sichtbar)."); }><Icon name="link" size=15/>"Link kopieren"</button>
                    <button class="menu-item danger" on:click=move |_| run(Box::pin(trash_files(vec![id])))><Icon name="trash" size=15/>"Löschen"</button>
                }.into_any()
            }}
        </Dropdown>
    }
}

fn select_box(id: Uuid) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    view! {
        <input type="checkbox" class="item-check"
            prop:checked=move || ctx.selected.with(|s| s.contains(&id))
            on:click=|ev| ev.stop_propagation()
            on:change=move |_| ctx.selected.update(|s| { if !s.remove(&id) { s.insert(id); } })/>
    }
}

#[component]
fn FolderCard(f: FolderView) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let id = f.id;
    let in_trash = ctx.trash.get_untracked();
    view! {
        <div class="file-card folder-card" on:dblclick=move |_| if !in_trash { ctx.open_folder.run(id) }>
            <button class="file-card-main" on:click=move |_| if !in_trash { ctx.open_folder.run(id) }>
                <span class="file-icon kind-folder" style="width:56px;height:56px"><Icon name="folder" size=28/></span>
                <span class="file-name">{f.name.clone()}</span>
                <span class="muted small">{format!("{} Elemente", f.item_count)}</span>
            </button>
            <div class="file-card-menu">{folder_actions(f.clone())}</div>
        </div>
    }
}

#[component]
fn FileCard(f: FileView) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let id = f.id;
    let open = f.clone();
    view! {
        <div class="file-card" class:selected=move || ctx.selected.with(|s| s.contains(&id))>
            <div class="file-card-check">{select_box(id)}</div>
            <button class="file-card-main" on:click=move |_| ctx.open_file.run(open.clone())>
                <FileThumb file=f.clone() size=96/>
                <span class="file-name" title=f.name.clone()>{f.name.clone()}</span>
                <span class="muted small">
                    <Icon name=f.visibility.icon() size=12/>" "{format_bytes(f.size_bytes)}
                </span>
            </button>
            <div class="file-card-menu">{file_actions(f.clone())}</div>
        </div>
    }
}

#[component]
fn FolderRow(f: FolderView) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let id = f.id;
    view! {
        <tr>
            <td></td>
            <td><button class="link-btn file-row-name" on:click=move |_| if !ctx.trash.get_untracked() { ctx.open_folder.run(id) }>
                <Icon name="folder" size=18/>{f.name.clone()}</button></td>
            <td class="small muted">{format!("{} Elemente", f.item_count)}</td>
            <td class="small muted"><TimeAgo ts=f.created_at/></td>
            <td></td>
            <td>{folder_actions(f.clone())}</td>
        </tr>
    }
}

#[component]
fn FileRow(f: FileView) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let id = f.id;
    let open = f.clone();
    view! {
        <tr class:selected=move || ctx.selected.with(|s| s.contains(&id))>
            <td>{select_box(id)}</td>
            <td><button class="link-btn file-row-name" on:click=move |_| ctx.open_file.run(open.clone())>
                <FileThumb file=f.clone() size=28/>{f.name.clone()}</button></td>
            <td class="small muted nowrap">{format_bytes(f.size_bytes)}</td>
            <td class="small muted nowrap"><TimeAgo ts=f.updated_at/></td>
            <td class="small" title=f.visibility.label()><Icon name=f.visibility.icon() size=15/></td>
            <td>{file_actions(f.clone())}</td>
        </tr>
    }
}

#[component]
fn NewFolderModal(open: RwSignal<bool>, parent: Signal<Option<Uuid>>) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let toasts = use_toasts();
    let name = RwSignal::new(String::new());
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let n = name.get_untracked();
        leptos::task::spawn_local(async move {
            match create_folder(parent.get_untracked(), n).await {
                Ok(_) => {
                    open.set(false);
                    name.set(String::new());
                    ctx.reload.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Modal open=open title="Neuer Ordner">
            <form class="stack" on:submit=submit>
                <input type="text" placeholder="Ordnername" required maxlength="200" bind:value=name autofocus=true/>
                <div class="row end"><button class="btn btn-primary">"Erstellen"</button></div>
            </form>
        </Modal>
    }
}

#[component]
fn RenameModal(target: RwSignal<Option<Target>>) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let toasts = use_toasts();
    let open = RwSignal::new(false);
    let name = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    Effect::new(move |_| {
        if let Some(t) = target.get() {
            match &t {
                Target::File(f) => {
                    name.set(f.name.clone());
                    description.set(f.description.clone());
                }
                Target::Folder(f) => {
                    name.set(f.name.clone());
                    description.set(String::new());
                }
            }
            open.set(true);
        }
    });
    Effect::new(move |_| {
        if !open.get() {
            target.set(None);
        }
    });
    let is_file = move || matches!(target.get(), Some(Target::File(_)));
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(t) = target.get_untracked() else { return };
        let (n, d) = (name.get_untracked(), description.get_untracked());
        leptos::task::spawn_local(async move {
            let res = match t {
                Target::File(f) => rename_file(f.id, n, d).await,
                Target::Folder(f) => rename_folder(f.id, n).await,
            };
            match res {
                Ok(()) => {
                    open.set(false);
                    ctx.reload.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Modal open=open title="Umbenennen">
            <form class="stack" on:submit=submit>
                <label class="field"><span>"Name"</span><input type="text" required maxlength="200" bind:value=name/></label>
                <Show when=is_file>
                    <label class="field"><span>"Beschreibung"</span><textarea rows=3 bind:value=description></textarea></label>
                </Show>
                <div class="row end"><button class="btn btn-primary">"Speichern"</button></div>
            </form>
        </Modal>
    }
}

#[component]
fn MoveModal(ids: RwSignal<Option<Vec<Uuid>>>, folder: RwSignal<Option<FolderView>>) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    let toasts = use_toasts();
    let open = RwSignal::new(false);
    let target = RwSignal::new(String::new());
    Effect::new(move |_| {
        if ids.get().is_some() || folder.get().is_some() {
            target.set(String::new());
            open.set(true);
        }
    });
    Effect::new(move |_| {
        if !open.get() {
            ids.set(None);
            folder.set(None);
        }
    });
    let tree = Resource::new(move || open.get(), |o| async move { if o { folder_tree().await } else { Ok(Vec::new()) } });
    let submit = move |_| {
        let dest = Uuid::parse_str(&target.get_untracked()).ok();
        let file_ids = ids.get_untracked();
        let moving_folder = folder.get_untracked();
        leptos::task::spawn_local(async move {
            let res = match (file_ids, moving_folder) {
                (Some(ids), _) => move_files(ids, dest).await,
                (None, Some(f)) => move_folder(f.id, dest).await,
                _ => Ok(()),
            };
            match res {
                Ok(()) => {
                    open.set(false);
                    ctx.selected.set(HashSet::new());
                    ctx.reload.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <Modal open=open title="Verschieben nach …">
            <div class="stack">
                <Suspense fallback=|| view! { <Spinner/> }>
                    {move || Suspend::new(async move {
                        let folders = tree.await.unwrap_or_default();
                        let excluded = folder.get_untracked().map(|f| f.id);
                        let ordered = order_tree(&folders, excluded);
                        view! {
                            <select bind:value=target size="10" class="folder-picker">
                                <option value="">"Meine Dateien (oberste Ebene)"</option>
                                {ordered.into_iter().map(|(f, depth)| view! {
                                    <option value=f.id.to_string()>{format!("{}{}", "\u{a0}\u{a0}\u{a0}".repeat(depth + 1), f.name)}</option>
                                }).collect_view()}
                            </select>
                        }
                    })}
                </Suspense>
                <div class="row end"><button class="btn btn-primary" on:click=submit>"Verschieben"</button></div>
            </div>
        </Modal>
    }
}

/// Depth-first order with depth, skipping `excluded` and its subtree.
fn order_tree(folders: &[FolderView], excluded: Option<Uuid>) -> Vec<(FolderView, usize)> {
    fn walk(all: &[FolderView], parent: Option<Uuid>, depth: usize, excluded: Option<Uuid>, out: &mut Vec<(FolderView, usize)>) {
        for f in all.iter().filter(|f| f.parent_id == parent) {
            if Some(f.id) == excluded || depth > 30 {
                continue;
            }
            out.push((f.clone(), depth));
            walk(all, Some(f.id), depth + 1, excluded, out);
        }
    }
    let mut out = Vec::new();
    walk(folders, None, 0, excluded, &mut out);
    out
}

#[component]
fn ShareModal(target: RwSignal<Option<Target>>) -> impl IntoView {
    let toasts = use_toasts();
    let open = RwSignal::new(false);
    let shares = RwSignal::new(Vec::<ShareView>::new());
    let expires = RwSignal::new("7".to_string());
    let password = RwSignal::new(String::new());
    let max_downloads = RwSignal::new(String::new());
    let ids = move || match target.get_untracked() {
        Some(Target::File(f)) => (Some(f.id), None),
        Some(Target::Folder(f)) => (None, Some(f.id)),
        None => (None, None),
    };
    let load = move || {
        let (file, folder) = ids();
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_shares(file, folder).await {
                shares.set(list);
            }
        });
    };
    Effect::new(move |_| {
        if target.get().is_some() {
            shares.set(Vec::new());
            open.set(true);
            load();
        }
    });
    Effect::new(move |_| {
        if !open.get() {
            target.set(None);
        }
    });
    let create = move |_| {
        let (file, folder) = ids();
        let days = expires.get_untracked().parse::<i64>().ok().filter(|d| *d > 0);
        let pw = Some(password.get_untracked()).filter(|p| !p.is_empty());
        let max = max_downloads.get_untracked().parse::<i32>().ok();
        leptos::task::spawn_local(async move {
            match create_share(file, folder, days, pw, max).await {
                Ok(s) => {
                    copy_to_clipboard(&format!("{}{}", origin(), s.path()));
                    toasts.success("Freigabe erstellt – Link kopiert.");
                    password.set(String::new());
                    load();
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let title = move || match target.get() {
        Some(Target::File(f)) => format!("„{}“ teilen", f.name),
        Some(Target::Folder(f)) => format!("Ordner „{}“ teilen", f.name),
        None => "Teilen".into(),
    };
    view! {
        <Modal open=open title="Freigabe-Links" wide=false>
            <div class="stack">
                <p class="muted small">{title}" – jeder mit dem Link kann die Datei ansehen und herunterladen, auch ohne Konto."</p>
                <Show when=move || !shares.with(|s| s.is_empty())>
                    <ul class="share-list">
                        <For each=move || shares.get() key=|s| s.id children=move |s| {
                            let id = s.id;
                            let url = format!("{}{}", origin(), s.path());
                            let url2 = url.clone();
                            view! {
                                <li>
                                    <input type="text" readonly prop:value=url.clone()/>
                                    <div class="row gap-sm small muted">
                                        {s.expires_at.map(|t| view! { <span>"läuft ab " <TimeAgo ts=t/></span> })}
                                        {s.has_password.then_some("· mit Passwort")}
                                        {format!("· {} Downloads", s.download_count)}
                                        {s.max_downloads.map(|m| format!(" von {m}"))}
                                        <span class="grow"></span>
                                        <button class="btn btn-sm" on:click=move |_| { copy_to_clipboard(&url2); toasts.success("Kopiert."); }><Icon name="copy" size=14/></button>
                                        <button class="btn btn-sm btn-danger-soft" on:click=move |_| {
                                            leptos::task::spawn_local(async move {
                                                if revoke_share(id).await.is_ok() { shares.update(|v| v.retain(|x| x.id != id)); }
                                            });
                                        }>"Widerrufen"</button>
                                    </div>
                                </li>
                            }
                        }/>
                    </ul>
                </Show>
                <div class="card stack-sm">
                    <strong>"Neuer Link"</strong>
                    <div class="row gap wrap">
                        <label class="field grow"><span>"Gültig"</span>
                            <select bind:value=expires>
                                <option value="1">"1 Tag"</option>
                                <option value="7">"7 Tage"</option>
                                <option value="30">"30 Tage"</option>
                                <option value="0">"Unbegrenzt"</option>
                            </select>
                        </label>
                        <label class="field grow"><span>"Max. Downloads"</span><input type="number" min="1" placeholder="unbegrenzt" bind:value=max_downloads/></label>
                    </div>
                    <label class="field"><span>"Passwort (optional)"</span><input type="text" bind:value=password autocomplete="off"/></label>
                    <div class="row end"><button class="btn btn-primary" on:click=create><Icon name="link" size=16/>"Link erstellen"</button></div>
                </div>
            </div>
        </Modal>
    }
}

/// Inline preview for images, video, audio, PDF and text.
#[component]
pub fn FilePreview(file: FileView, #[prop(optional, into)] share: Option<String>) -> impl IntoView {
    let suffix = share.map(|s| format!("?s={s}")).unwrap_or_default();
    let url = format!("{}{suffix}", file.url());
    match file.kind() {
        FileKind::Image => view! { <img class="preview-media" src=format!("{}{suffix}", file.preview_url()) alt=file.name.clone()/> }.into_any(),
        FileKind::Video => view! { <video class="preview-media" src=url controls=true preload="metadata"></video> }.into_any(),
        FileKind::Audio => view! { <audio class="preview-audio" src=url controls=true preload="metadata"></audio> }.into_any(),
        FileKind::Pdf => view! { <iframe class="preview-pdf" src=url title=file.name.clone()></iframe> }.into_any(),
        FileKind::Text if file.size_bytes < 512 * 1024 => {
            let text = RwSignal::new(None::<String>);
            #[cfg(feature = "hydrate")]
            {
                let u = url.clone();
                leptos::task::spawn_local(async move {
                    if let Ok(resp) = gloo_fetch_text(&u).await {
                        text.set(Some(resp));
                    }
                });
            }
            view! { <pre class="preview-text">{move || text.get().unwrap_or_else(|| "Lade …".into())}</pre> }.into_any()
        }
        _ => view! { <div class="preview-none"><Icon name=file.kind().icon() size=56/><p class="muted">"Keine Vorschau verfügbar."</p></div> }.into_any(),
    }
}

#[cfg(feature = "hydrate")]
async fn gloo_fetch_text(url: &str) -> Result<String, ()> {
    use wasm_bindgen::JsCast;
    let resp = wasm_bindgen_futures::JsFuture::from(window().fetch_with_str(url)).await.map_err(|_| ())?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| ())?;
    let text = wasm_bindgen_futures::JsFuture::from(resp.text().map_err(|_| ())?).await.map_err(|_| ())?;
    text.as_string().ok_or(())
}

#[component]
fn PreviewModal(open: RwSignal<bool>, file: RwSignal<Option<FileView>>) -> impl IntoView {
    let ctx = expect_context::<DriveCtx>();
    view! {
        <Modal open=open title=Signal::derive(move || file.get().map(|f| f.name).unwrap_or_default()) wide=true>
            {move || file.get().map(|f| {
                let share_f = f.clone();
                let rename_f = f.clone();
                view! {
                    <div class="preview">
                        <div class="preview-stage"><FilePreview file=f.clone()/></div>
                        <aside class="preview-info">
                            <dl>
                                <dt>"Größe"</dt><dd>{format_bytes(f.size_bytes)}</dd>
                                <dt>"Typ"</dt><dd>{f.mime_type.clone()}</dd>
                                {f.width.zip(f.height).map(|(w, h)| view! { <dt>"Abmessungen"</dt><dd>{format!("{w} × {h}")}</dd> })}
                                {f.duration_secs.map(|d| view! { <dt>"Dauer"</dt><dd>{shared::util::format_duration(d)}</dd> })}
                                <dt>"Hochgeladen"</dt><dd><TimeAgo ts=f.created_at absolute=true/></dd>
                                <dt>"Sichtbarkeit"</dt><dd><Icon name=f.visibility.icon() size=14/>" "{f.visibility.label()}</dd>
                                <dt>"Downloads"</dt><dd>{f.download_count}</dd>
                            </dl>
                            {(!f.description.is_empty()).then(|| view! { <p class="small">{f.description.clone()}</p> })}
                            <div class="stack-sm">
                                <a class="btn btn-primary" href=f.download_url() rel="external" download=""><Icon name="download" size=16/>"Herunterladen"</a>
                                <button class="btn" on:click=move |_| { open.set(false); ctx.share_target.set(Some(Target::File(share_f.clone()))); }><Icon name="share" size=16/>"Teilen"</button>
                                <button class="btn" on:click=move |_| { open.set(false); ctx.rename_target.set(Some(Target::File(rename_f.clone()))); }><Icon name="edit" size=16/>"Umbenennen"</button>
                            </div>
                        </aside>
                    </div>
                }
            })}
        </Modal>
    }
}

/// Public share page `/s/:token`.
#[component]
pub fn SharePage() -> impl IntoView {
    let params = use_params_map();
    let token = move || params.read().get("token").unwrap_or_default();
    let info = RwSignal::new(None::<Result<ShareInfo, crate::error::AppError>>);
    let data = Resource::new(token, get_share_info);
    view! {
        <Title text="Freigabe"/>
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                let res = data.await;
                info.set(Some(res));
                view! { <ShareContent info=info token=token()/> }
            })}
        </Suspense>
    }
}

#[component]
fn ShareContent(info: RwSignal<Option<Result<ShareInfo, crate::error::AppError>>>, token: String) -> impl IntoView {
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let tok = token.clone();
    let unlock = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let t = tok.clone();
        leptos::task::spawn_local(async move {
            match unlock_share(t, password.get_untracked()).await {
                Ok(i) => info.set(Some(Ok(i))),
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };
    move || match info.get() {
        None => view! { <Spinner/> }.into_any(),
        Some(Err(_)) => view! {
            <div class="empty-state big"><Icon name="link" size=40/><p>"Diese Freigabe existiert nicht oder ist abgelaufen."</p></div>
        }.into_any(),
        Some(Ok(i)) if i.locked => {
            let unlock = unlock.clone();
            view! {
                <div class="auth-card card">
                    <h1><Icon name="lock"/>"Geschützte Freigabe"</h1>
                    <p class="muted">{i.owner.display_name.clone()} " hat diese Freigabe mit einem Passwort geschützt."</p>
                    <form class="stack" on:submit=unlock>
                        <input type="password" placeholder="Passwort" required bind:value=password/>
                        {move || error.get().map(|e| view! { <div class="alert alert-error">{e}</div> })}
                        <button class="btn btn-primary btn-block">"Öffnen"</button>
                    </form>
                </div>
            }.into_any()
        }
        Some(Ok(i)) => {
            let t = token.clone();
            view! {
                <div class="share-page">
                    <div class="row gap share-owner">
                        <Avatar user=i.owner.clone() size=36/>
                        <div>
                            <div><strong>{i.owner.display_name.clone()}</strong>" hat etwas mit dir geteilt"</div>
                            {i.expires_at.map(|t| view! { <div class="muted small">"Verfügbar bis " <TimeAgo ts=t absolute=true/></div> })}
                        </div>
                    </div>
                    {i.file.clone().map(|f| {
                        let dl = format!("{}?s={}", f.download_url(), t);
                        view! {
                            <div class="card stack">
                                <h1 class="ellipsis">{f.name.clone()}</h1>
                                <p class="muted">{format_bytes(f.size_bytes)} " · " {f.mime_type.clone()}</p>
                                <div class="share-preview"><FilePreview file=f.clone() share=t.clone()/></div>
                                <a class="btn btn-primary" href=dl rel="external" download=""><Icon name="download" size=16/>"Herunterladen"</a>
                            </div>
                        }
                    })}
                    {i.folder.clone().map(|folder| view! {
                        <div class="card stack">
                            <h1><Icon name="folder"/>{folder.name.clone()}</h1>
                            {if i.files.is_empty() {
                                view! { <p class="muted">"Dieser Ordner ist leer."</p> }.into_any()
                            } else {
                                view! {
                                    <ul class="share-files">
                                        {i.files.iter().map(|f| {
                                            let dl = format!("{}?s={}", f.download_url(), t);
                                            let open = format!("{}?s={}", f.url(), t);
                                            view! {
                                                <li class="row gap">
                                                    <FileThumb file=f.clone() size=36/>
                                                    <a class="grow ellipsis" href=open rel="external" target="_blank">{f.name.clone()}</a>
                                                    <span class="muted small">{format_bytes(f.size_bytes)}</span>
                                                    <a class="btn btn-sm" href=dl rel="external" download=""><Icon name="download" size=14/></a>
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ul>
                                }.into_any()
                            }}
                        </div>
                    })}
                </div>
            }.into_any()
        }
    }
}
