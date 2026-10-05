//! Gallery: album overview, album page with uploads and ordering, image
//! page with EXIF, reactions and comments, and a keyboard-driven lightbox.

use leptos::prelude::*;
use leptos_meta::{Meta, Title};
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use shared::files::{FilePurpose, FileView, Visibility};
use shared::gallery::{AlbumFilter, AlbumInput, AlbumView, ImageView, ALBUMS_PER_PAGE};
use shared::util::format_bytes;
use shared::SubjectType;
use uuid::Uuid;

use crate::api::gallery::*;
use crate::components::uploads::{UploadQueue, UploadQueueView};
use crate::components::comments::CommentsSection;
use crate::components::common::{confirm, copy_to_clipboard, origin, sleep_ms};
use crate::components::{Avatar, EmptyState, ErrorBox, Icon, Modal, Pager, ReactionBar, ReportButton, Spinner, TimeAgo};
use crate::pages::not_found::Missing;
use crate::state::{use_session, use_toasts};
use crate::upload::UploadOpts;

const SLIDESHOW_MS: u32 = 4500;

fn visibility_icon(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "lock",
        Visibility::Members => "users",
        Visibility::Public => "globe",
    }
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Albums,
    Latest,
    Mine,
}

#[component]
pub fn GalleryIndex() -> impl IntoView {
    let session = use_session();
    let query = use_query_map();
    let navigate = use_navigate();
    let user_filter = Memo::new(move |_| query.with(|q| q.get("user").and_then(|s| Uuid::parse_str(&s).ok())));
    let tab = Memo::new(move |_| match query.with(|q| q.get("tab")).as_deref() {
        Some("latest") => Tab::Latest,
        Some("mine") => Tab::Mine,
        _ => Tab::Albums,
    });
    let page = RwSignal::new(0i64);
    Effect::new(move |_| {
        tab.track();
        user_filter.track();
        page.set(0);
    });
    let create_open = RwSignal::new(false);
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let on_created = Callback::new(move |a: AlbumView| {
        create_open.set(false);
        go.run(a.url());
    });
    let tab_href = move |t: &str| match user_filter.get() {
        Some(u) => format!("/gallery?tab={t}&user={u}"),
        None => format!("/gallery?tab={t}"),
    };

    view! {
        <Title text="Galerie"/>
        <div class="page-header">
            <div>
                <h1>"Galerie"</h1>
                <p class="muted">"Fotos und Alben aus der Community"</p>
            </div>
            <div class="page-actions">
                <Show when=move || session.logged_in()>
                    <button class="btn btn-primary" on:click=move |_| create_open.set(true)>
                        <Icon name="plus" size=16/>"Neues Album"
                    </button>
                </Show>
            </div>
        </div>
        <div class="tabs">
            <a class="tab" class:active=move || tab.get() == Tab::Albums href=move || tab_href("albums")><Icon name="layers" size=16/>"Alben"</a>
            <a class="tab" class:active=move || tab.get() == Tab::Latest href=move || tab_href("latest")><Icon name="image" size=16/>"Neueste Bilder"</a>
            <Show when=move || session.logged_in()>
                <a class="tab" class:active=move || tab.get() == Tab::Mine href="/gallery?tab=mine"><Icon name="user" size=16/>"Meine Alben"</a>
            </Show>
        </div>
        <Show when=move || user_filter.get().is_some()>
            <div class="alert small">
                <Icon name="user" size=14/>"Gefiltert nach Mitglied. "
                <a href=move || format!("/gallery?tab={}", match tab.get() { Tab::Latest => "latest", _ => "albums" })>"Filter entfernen"</a>
            </div>
        </Show>
        {move || match tab.get() {
            Tab::Latest => view! { <LatestImages user=user_filter.get()/> }.into_any(),
            t => {
                let filter = match (t, user_filter.get()) {
                    (Tab::Mine, _) => AlbumFilter::Mine,
                    (_, Some(u)) => AlbumFilter::User(u),
                    _ => AlbumFilter::All,
                };
                view! { <AlbumGrid filter=filter page=page/> }.into_any()
            }
        }}
        <Modal open=create_open title="Neues Album">
            <AlbumForm album=None on_saved=on_created/>
        </Modal>
    }
}

#[component]
fn AlbumGrid(filter: AlbumFilter, page: RwSignal<i64>) -> impl IntoView {
    let albums = Resource::new(move || (filter.clone(), page.get()), |(f, p)| list_albums(f, p + 1));
    let total = RwSignal::new(0i64);
    view! {
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match albums.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(p) if p.items.is_empty() => view! {
                        <EmptyState icon="image" title="Noch keine Alben">
                            <p class="muted">"Lege ein Album an und lade die ersten Bilder hoch."</p>
                        </EmptyState>
                    }.into_any(),
                    Ok(p) => {
                        total.set(p.total);
                        view! {
                            <div class="album-grid">
                                {p.items.into_iter().map(|a| view! { <AlbumCard album=a/> }).collect_view()}
                            </div>
                        }.into_any()
                    }
                }
            })}
        </Transition>
        <Pager page=page total=total per_page=ALBUMS_PER_PAGE/>
    }
}

#[component]
fn AlbumCard(album: AlbumView) -> impl IntoView {
    view! {
        <a class="album-card card" href=album.url()>
            <div class="album-cover">
                {match album.cover_url.clone() {
                    Some(c) => view! { <img src=c alt="" loading="lazy"/> }.into_any(),
                    None => view! { <div class="album-cover-empty"><Icon name="image" size=36/></div> }.into_any(),
                }}
                <span class="album-count"><Icon name="image" size=12/>{album.image_count}</span>
            </div>
            <div class="album-meta">
                <strong class="ellipsis">{album.title.clone()}</strong>
                <div class="muted small row gap-sm">
                    <Avatar user=album.owner.clone() size=16/>
                    <span class="ellipsis">{album.owner.display_name.clone()}</span>
                    <span class="grow"></span>
                    {album.is_community.then(|| view! { <span title="Community-Album"><Icon name="users" size=13/></span> })}
                    <span title=album.visibility.label()><Icon name=visibility_icon(album.visibility) size=13/></span>
                </div>
            </div>
        </a>
    }
}

#[component]
fn LatestImages(user: Option<Uuid>) -> impl IntoView {
    let images = RwSignal::new(Vec::<ImageView>::new());
    let cursor = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let done = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let lightbox = RwSignal::new(None::<usize>);
    let load_more = move || {
        if loading.get_untracked() || done.get_untracked() {
            return;
        }
        loading.set(true);
        leptos::task::spawn_local(async move {
            match latest_images(user, cursor.get_untracked()).await {
                Ok(p) => {
                    done.set(p.next_cursor.is_none());
                    cursor.set(p.next_cursor);
                    images.update(|v| v.extend(p.items));
                }
                Err(e) => error.set(Some(e.to_string())),
            }
            loading.set(false);
        });
    };
    Effect::new(move |_| load_more());
    view! {
        {move || error.get().map(|e| view! { <div class="error">{e}</div> })}
        <Show when=move || !images.with(|v| v.is_empty()) fallback=move || view! {
            <Show when=move || !loading.get() fallback=|| view! { <Spinner/> }>
                <EmptyState icon="image" title="Noch keine Bilder"/>
            </Show>
        }>
            <ImageGrid images=images on_open=Callback::new(move |i| lightbox.set(Some(i)))/>
        </Show>
        <Show when=move || !done.get() && !images.with(|v| v.is_empty())>
            <div class="center">
                <button class="btn" disabled=move || loading.get() on:click=move |_| load_more()>"Mehr laden"</button>
            </div>
        </Show>
        <Lightbox images=images index=lightbox/>
    }
}

// ---------------------------------------------------------------------------
// Image grid + lightbox
// ---------------------------------------------------------------------------

#[component]
fn ImageGrid(images: RwSignal<Vec<ImageView>>, on_open: Callback<usize>) -> impl IntoView {
    let entries = move || images.get().into_iter().enumerate().collect::<Vec<_>>();
    let entry_key = |e: &(usize, ImageView)| (e.0, e.1.id);
    view! {
        <div class="image-grid">
            <For
                each=entries
                key=entry_key
                children=move |(i, img): (usize, ImageView)| {
                    let title = img.display_title();
                    let alt = title.clone();
                    view! {
                        <a class="image-tile" href=img.url() title=title
                            on:click=move |ev: leptos::ev::MouseEvent| {
                                if !ev.ctrl_key() && !ev.meta_key() && !ev.shift_key() {
                                    ev.prevent_default();
                                    on_open.run(i);
                                }
                            }
                        >
                            <img src=img.thumb_url() alt=alt loading="lazy"/>
                            <span class="image-tile-info">
                                {(img.like_count > 0).then(|| view! { <span><Icon name="heart" size=12/>{img.like_count}</span> })}
                                {(img.comment_count > 0).then(|| view! { <span><Icon name="message-circle" size=12/>{img.comment_count}</span> })}
                            </span>
                        </a>
                    }
                }
            />
        </div>
    }
}

/// Fullscreen viewer: ←/→ navigate, Space toggles the slideshow, Esc closes.
#[component]
pub fn Lightbox(images: RwSignal<Vec<ImageView>>, index: RwSignal<Option<usize>>) -> impl IntoView {
    let playing = RwSignal::new(false);
    let touch_x = StoredValue::new(None::<i32>);
    let count = move || images.with(|v| v.len());
    let step = move |delta: i64| {
        let n = count() as i64;
        if n == 0 {
            return;
        }
        index.update(|i| {
            if let Some(cur) = i {
                *cur = ((*cur as i64 + delta).rem_euclid(n)) as usize;
            }
        });
    };
    let close = move || {
        playing.set(false);
        index.set(None);
    };
    let current = move || index.get().and_then(|i| images.with(|v| v.get(i).cloned()));

    #[cfg(feature = "hydrate")]
    {
        let handle = window_event_listener(leptos::ev::keydown, move |ev| {
            if index.get_untracked().is_none() {
                return;
            }
            match ev.key().as_str() {
                "ArrowRight" => step(1),
                "ArrowLeft" => step(-1),
                "Escape" => close(),
                " " => {
                    ev.prevent_default();
                    playing.update(|p| *p = !*p);
                }
                _ => {}
            }
        });
        on_cleanup(move || handle.remove());
    }

    // Slideshow: advance after a pause unless the image changed meanwhile.
    Effect::new(move |_| {
        let i = index.get();
        if !playing.get() || i.is_none() {
            return;
        }
        leptos::task::spawn_local(async move {
            sleep_ms(SLIDESHOW_MS).await;
            if playing.get_untracked() && index.get_untracked() == i {
                step(1);
            }
        });
    });

    view! {
        <Show when=move || index.get().is_some()>
            <div class="lightbox" role="dialog" aria-modal="true"
                on:click=move |_| close()
                on:touchstart=move |ev: leptos::ev::TouchEvent| {
                    if let Some(t) = ev.touches().get(0) { touch_x.set_value(Some(t.client_x())); }
                }
                on:touchend=move |ev: leptos::ev::TouchEvent| {
                    let start = touch_x.get_value();
                    touch_x.set_value(None);
                    if let (Some(s), Some(t)) = (start, ev.changed_touches().get(0)) {
                        let dx = t.client_x() - s;
                        if dx.abs() > 50 { step(if dx < 0 { 1 } else { -1 }); }
                    }
                }
            >
                {move || current().map(|img| {
                    let next_preload = index.get().and_then(|i| images.with(|v| v.get((i + 1) % v.len().max(1)).map(|n| n.preview_url())));
                    let title = img.display_title();
                    view! {
                        <div class="lightbox-top" on:click=|ev| ev.stop_propagation()>
                            <span class="lightbox-count">{move || format!("{} / {}", index.get().unwrap_or(0) + 1, count())}</span>
                            <span class="grow ellipsis">{title.clone()}</span>
                            <button class="lb-btn" title=move || if playing.get() { "Diashow anhalten (Leertaste)" } else { "Diashow (Leertaste)" }
                                on:click=move |_| playing.update(|p| *p = !*p)>
                                {move || view! { <Icon name=if playing.get() { "pause" } else { "play" } size=18/> }}
                            </button>
                            <a class="lb-btn" href=img.download_url() title="Original herunterladen" rel="external"><Icon name="download" size=18/></a>
                            <a class="lb-btn" href=img.url() title="Details und Kommentare"><Icon name="message-circle" size=18/></a>
                            <button class="lb-btn" title="Schließen (Esc)" on:click=move |_| close()><Icon name="x" size=20/></button>
                        </div>
                        <button class="lb-nav lb-prev" title="Vorheriges (←)" on:click=move |ev| { ev.stop_propagation(); step(-1); }>
                            <Icon name="chevron-left" size=32/>
                        </button>
                        <img class="lightbox-img" src=img.preview_url() alt=title.clone() on:click=|ev| ev.stop_propagation()/>
                        <button class="lb-nav lb-next" title="Nächstes (→)" on:click=move |ev| { ev.stop_propagation(); step(1); }>
                            <Icon name="chevron-right" size=32/>
                        </button>
                        {(!img.description.is_empty()).then(|| view! {
                            <div class="lightbox-caption" on:click=|ev| ev.stop_propagation()>{img.description.clone()}</div>
                        })}
                        {next_preload.map(|u| view! { <link rel="prefetch" href=u/> })}
                    }
                })}
            </div>
        </Show>
    }
}

// ---------------------------------------------------------------------------
// Album form
// ---------------------------------------------------------------------------

#[component]
fn AlbumForm(album: Option<AlbumView>, on_saved: Callback<AlbumView>) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let id = album.as_ref().map(|a| a.id);
    let title = RwSignal::new(album.as_ref().map(|a| a.title.clone()).unwrap_or_default());
    let description = RwSignal::new(album.as_ref().map(|a| a.description.clone()).unwrap_or_default());
    let visibility = RwSignal::new(album.as_ref().map(|a| a.visibility).unwrap_or(Visibility::Public));
    let community = RwSignal::new(album.as_ref().is_some_and(|a| a.is_community));
    let saving = RwSignal::new(false);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = AlbumInput {
            title: title.get_untracked(),
            description: description.get_untracked(),
            visibility: visibility.get_untracked(),
            is_community: community.get_untracked(),
        };
        saving.set(true);
        leptos::task::spawn_local(async move {
            let res = match id {
                Some(id) => update_album(id, input).await,
                None => create_album(input).await,
            };
            saving.set(false);
            match res {
                Ok(a) => {
                    toasts.success("Album gespeichert.");
                    on_saved.run(a);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <form class="form" on:submit=submit>
            <label class="field">
                <span>"Titel"</span>
                <input type="text" maxlength="120" required bind:value=title placeholder="z. B. Sommerfest 2026"/>
            </label>
            <label class="field">
                <span>"Beschreibung"</span>
                <textarea rows="3" maxlength="5000" bind:value=description></textarea>
            </label>
            <label class="field">
                <span>"Sichtbarkeit"</span>
                <select prop:value=move || visibility.get().as_str().to_string()
                    on:change=move |ev| visibility.set(Visibility::parse(&event_target_value(&ev)))>
                    <option value="public">"Öffentlich – für alle sichtbar"</option>
                    <option value="members">"Mitglieder – nur angemeldete Nutzer"</option>
                    <option value="private" disabled=move || community.get()>"Privat – nur für dich"</option>
                </select>
            </label>
            <Show when=move || session.is_mod()>
                <label class="check">
                    <input type="checkbox" bind:checked=community
                        on:change=move |_| if community.get_untracked() && visibility.get_untracked() == Visibility::Private {
                            visibility.set(Visibility::Members);
                        }/>
                    <span>"Community-Album – alle Mitglieder dürfen Bilder hinzufügen"</span>
                </label>
            </Show>
            <div class="form-actions">
                <button class="btn btn-primary" type="submit" disabled=move || saving.get()>
                    {if id.is_some() { "Speichern" } else { "Album anlegen" }}
                </button>
            </div>
        </form>
    }
}

// ---------------------------------------------------------------------------
// Album page
// ---------------------------------------------------------------------------

#[component]
pub fn AlbumPage() -> impl IntoView {
    let params = use_params_map();
    let album_id = Memo::new(move |_| params.with(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok())));
    let version = RwSignal::new(0u32);
    let detail = Resource::new(
        move || (album_id.get(), version.get()),
        |(id, _)| async move {
            match id {
                Some(id) => get_album(id).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let reload = Callback::new(move |_| version.update(|v| *v += 1));
    view! {
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match detail.await {
                    Ok(None) => view! { <Missing text="Dieses Album gibt es nicht."/> }.into_any(),
                    Err(e) if e.is_not_found() => view! { <Missing text="Dieses Album gibt es nicht (mehr) oder es ist nicht sichtbar."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(Some(d)) => view! { <AlbumBody album=d.album images=d.images reload=reload/> }.into_any(),
                }
            })}
        </Transition>
    }
}

#[component]
fn AlbumBody(album: AlbumView, images: Vec<ImageView>, reload: Callback<()>) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let images = RwSignal::new(images);
    let lightbox = RwSignal::new(None::<usize>);
    let editing = RwSignal::new(false);
    let edit_open = RwSignal::new(false);
    let dragging = RwSignal::new(false);
    let queue = UploadQueue::new();
    let pending = RwSignal::new(0u32);
    let album_id = album.id;
    let can_edit = album.can_edit;
    let can_upload = album.can_upload;

    // Reload once a burst of uploads has settled.
    let on_uploaded = Callback::new(move |_f: FileView| {
        let n = pending.get_untracked() + 1;
        pending.set(n);
        leptos::task::spawn_local(async move {
            sleep_ms(700).await;
            if pending.get_untracked() == n && !queue.busy_untracked() {
                reload.run(());
            }
        });
    });
    let start_upload = move |files: Vec<web_sys::File>| {
        let opts = UploadOpts {
            purpose: FilePurpose::Gallery,
            context_id: Some(album_id),
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
        if can_upload {
            #[cfg(feature = "hydrate")]
            start_upload(crate::upload::files_from_drop(&ev));
        }
    };
    #[cfg(not(feature = "hydrate"))]
    let _ = start_upload;

    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let remove_album = move |_| {
        if !confirm("Album mit allen Bildern, Kommentaren und Reaktionen endgültig löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_album(album_id).await {
                Ok(()) => {
                    toasts.success("Album gelöscht.");
                    go.run("/gallery".into());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let move_image = move |from: usize, delta: i64| {
        let mut ids: Vec<Uuid> = images.with_untracked(|v| v.iter().map(|i| i.id).collect());
        let to = from as i64 + delta;
        if to < 0 || to as usize >= ids.len() {
            return;
        }
        ids.swap(from, to as usize);
        images.update(|v| v.swap(from, to as usize));
        leptos::task::spawn_local(async move {
            if let Err(e) = reorder_album(album_id, ids).await {
                toasts.error(e.to_string());
                reload.run(());
            }
        });
    };
    let make_cover = move |image_id: Uuid| {
        leptos::task::spawn_local(async move {
            match set_album_cover(album_id, Some(image_id)).await {
                Ok(()) => toasts.success("Titelbild gesetzt."),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let remove_image = move |image_id: Uuid| {
        if !confirm("Bild endgültig löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_image(image_id).await {
                Ok(()) => images.update(|v| v.retain(|i| i.id != image_id)),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let sort_by_date = move |_| {
        let mut list = images.get_untracked();
        list.sort_by_key(|i| i.taken_at.unwrap_or(i.created_at));
        let ids: Vec<Uuid> = list.iter().map(|i| i.id).collect();
        images.set(list);
        leptos::task::spawn_local(async move {
            if let Err(e) = reorder_album(album_id, ids).await {
                toasts.error(e.to_string());
            }
        });
    };
    let on_saved = Callback::new(move |_a: AlbumView| {
        edit_open.set(false);
        reload.run(());
    });
    let edit_album = album.clone();
    let start_slideshow = move |_| {
        if !images.with_untracked(|v| v.is_empty()) {
            lightbox.set(Some(0));
        }
    };

    view! {
        <Title text=album.title.clone()/>
        <Meta name="description" content=shared::util::truncate_chars(&album.description, 160)/>
        <div class="album-page"
            on:dragover=move |ev: leptos::ev::DragEvent| { ev.prevent_default(); if can_upload { dragging.set(true); } }
            on:dragleave=move |_| dragging.set(false)
            on:drop=on_drop
        >
            <div class="breadcrumbs">
                <a href="/gallery">"Galerie"</a><Icon name="chevron-right" size=14/>
                <a href=format!("/gallery?user={}", album.owner.id)>{album.owner.display_name.clone()}</a>
            </div>
            <div class="page-header">
                <div>
                    <h1>{album.title.clone()}</h1>
                    <div class="muted small row gap-sm wrap">
                        <Avatar user=album.owner.clone() size=18/>
                        <a class="user-link" href=album.owner.profile_url()>{album.owner.display_name.clone()}</a>
                        " · " {move || format!("{} Bilder", images.with(|v| v.len()))}
                        " · " <Icon name=visibility_icon(album.visibility) size=13/>{album.visibility.label()}
                        {album.is_community.then(|| view! { " · " <Icon name="users" size=13/>"Community-Album" })}
                        " · aktualisiert " <TimeAgo ts=album.updated_at/>
                    </div>
                </div>
                <div class="page-actions">
                    <button class="btn" on:click=start_slideshow disabled=move || images.with(|v| v.is_empty())>
                        <Icon name="play" size=16/>"Diashow"
                    </button>
                    {can_upload.then(|| view! {
                        <label class="btn btn-primary">
                            <Icon name="upload" size=16/>"Bilder hochladen"
                            <input type="file" multiple accept="image/*" class="hidden" on:change=on_pick/>
                        </label>
                    })}
                    {can_edit.then(|| view! {
                        <button class="btn" class:active=move || editing.get() on:click=move |_| editing.update(|e| *e = !*e)>
                            <Icon name="grid" size=16/>{move || if editing.get() { "Fertig" } else { "Anordnen" }}
                        </button>
                        <button class="btn" on:click=move |_| edit_open.set(true) title="Album bearbeiten"><Icon name="edit" size=16/></button>
                        <button class="btn btn-danger-soft" on:click=remove_album title="Album löschen"><Icon name="trash" size=16/></button>
                    })}
                </div>
            </div>
            {(!album.description.is_empty()).then(|| view! { <p class="album-description">{album.description.clone()}</p> })}
            <UploadQueueView queue=queue/>
            <Show when=move || dragging.get()>
                <div class="drop-hint"><Icon name="upload" size=28/>"Bilder hier ablegen"</div>
            </Show>
            <Show when=move || editing.get()>
                <div class="toolbar card">
                    <span class="muted small grow">"Mit den Pfeilen verschieben, ★ setzt das Titelbild."</span>
                    <button class="btn btn-sm" on:click=sort_by_date><Icon name="calendar" size=14/>"Nach Aufnahmedatum sortieren"</button>
                </div>
            </Show>
            <Show
                when=move || !images.with(|v| v.is_empty())
                fallback=move || view! {
                    <EmptyState icon="image" title="Dieses Album ist noch leer">
                        {can_upload.then(|| view! { <p class="muted">"Ziehe Bilder hierher oder nutze „Bilder hochladen“."</p> })}
                    </EmptyState>
                }
            >
                {move || if editing.get() {
                    view! {
                        <div class="image-grid editing">
                            {images.get().into_iter().enumerate().map(|(i, img)| {
                                let id = img.id;
                                view! {
                                    <div class="image-tile">
                                        <img src=img.thumb_url() alt="" loading="lazy"/>
                                        <div class="tile-tools">
                                            <button title="Nach vorne" on:click=move |_| move_image(i, -1)><Icon name="arrow-left" size=14/></button>
                                            <button title="Als Titelbild" on:click=move |_| make_cover(id)><Icon name="star" size=14/></button>
                                            <button title="Löschen" on:click=move |_| remove_image(id)><Icon name="trash" size=14/></button>
                                            <button title="Nach hinten" on:click=move |_| move_image(i, 1)><Icon name="arrow-right" size=14/></button>
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                } else {
                    view! { <ImageGrid images=images on_open=Callback::new(move |i| lightbox.set(Some(i)))/> }.into_any()
                }}
            </Show>
            <footer class="album-foot">
                <ReportButton subject_type=SubjectType::GalleryAlbum subject_id=album_id compact=true/>
            </footer>
        </div>
        <Lightbox images=images index=lightbox/>
        <Modal open=edit_open title="Album bearbeiten">
            <AlbumForm album=Some(edit_album.clone()) on_saved=on_saved/>
        </Modal>
    }
}

// ---------------------------------------------------------------------------
// Image page
// ---------------------------------------------------------------------------

#[component]
pub fn ImagePage() -> impl IntoView {
    let params = use_params_map();
    let navigate = use_navigate();
    let image_id = Memo::new(move |_| params.with(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok())));
    let version = RwSignal::new(0u32);
    let detail = Resource::new(
        move || (image_id.get(), version.get()),
        |(id, _)| async move {
            match id {
                Some(id) => get_image(id).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let neighbours = RwSignal::new((None::<Uuid>, None::<Uuid>));
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));

    #[cfg(feature = "hydrate")]
    {
        let handle = window_event_listener(leptos::ev::keydown, move |ev| {
            let target_is_input = ev
                .target()
                .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(t).ok())
                .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT") || el.is_content_editable());
            if target_is_input {
                return;
            }
            let (prev, next) = neighbours.get_untracked();
            match ev.key().as_str() {
                "ArrowLeft" => {
                    if let Some(p) = prev {
                        go.run(format!("/gallery/image/{p}"));
                    }
                }
                "ArrowRight" => {
                    if let Some(n) = next {
                        go.run(format!("/gallery/image/{n}"));
                    }
                }
                _ => {}
            }
        });
        on_cleanup(move || handle.remove());
    }

    view! {
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match detail.await {
                    Ok(None) => view! { <Missing text="Dieses Bild gibt es nicht."/> }.into_any(),
                    Err(e) if e.is_not_found() => view! { <Missing text="Dieses Bild gibt es nicht (mehr) oder es ist nicht sichtbar."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(Some(d)) => {
                        neighbours.set((d.prev, d.next));
                        view! { <ImageDetailView detail=d version=version go=go/> }.into_any()
                    }
                }
            })}
        </Transition>
    }
}

#[component]
fn ImageDetailView(detail: shared::gallery::ImageDetail, version: RwSignal<u32>, go: Callback<String>) -> impl IntoView {
    let toasts = use_toasts();
    let img = detail.image.clone();
    let album = detail.album.clone();
    let id = img.id;
    let editing = RwSignal::new(false);
    let title = RwSignal::new(img.title.clone());
    let description = RwSignal::new(img.description.clone());
    let target_album = RwSignal::new(img.album_id.to_string());
    let albums = RwSignal::new(Vec::<AlbumView>::new());
    let saving = RwSignal::new(false);
    let lightbox = RwSignal::new(None::<usize>);
    let single = RwSignal::new(vec![img.clone()]);

    let start_edit = move |_| {
        editing.set(true);
        if albums.with_untracked(|a| a.is_empty()) {
            leptos::task::spawn_local(async move {
                if let Ok(list) = upload_albums().await {
                    albums.set(list);
                }
            });
        }
    };
    let current_album = img.album_id;
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let target = Uuid::parse_str(&target_album.get_untracked()).ok().filter(|t| *t != current_album);
        saving.set(true);
        leptos::task::spawn_local(async move {
            let res = update_image(id, title.get_untracked(), description.get_untracked(), target).await;
            saving.set(false);
            match res {
                Ok(()) => {
                    toasts.success("Gespeichert.");
                    editing.set(false);
                    version.update(|v| *v += 1);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let album_url = album.url();
    let remove = move |_| {
        if !confirm("Dieses Bild endgültig löschen?") {
            return;
        }
        let album_url = album_url.clone();
        leptos::task::spawn_local(async move {
            match delete_image(id).await {
                Ok(()) => {
                    toasts.success("Bild gelöscht.");
                    go.run(album_url);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let set_cover = move |_| {
        leptos::task::spawn_local(async move {
            match set_album_cover(current_album, Some(id)).await {
                Ok(()) => toasts.success("Titelbild gesetzt."),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let share_url = img.url();
    let display_title = img.display_title();
    let meta_description = if img.description.is_empty() {
        format!("Bild aus dem Album „{}“", album.title)
    } else {
        shared::util::truncate_chars(&img.description, 160)
    };
    let dims = match (img.width, img.height) {
        (Some(w), Some(h)) => format!("{w} × {h} px · "),
        _ => String::new(),
    };

    view! {
        <Title text=display_title.clone()/>
        <Meta name="description" content=meta_description/>
        <Meta property="og:image" content=img.preview_url()/>
        <div class="breadcrumbs">
            <a href="/gallery">"Galerie"</a><Icon name="chevron-right" size=14/>
            <a href=album.url()>{album.title.clone()}</a>
        </div>
        <div class="image-page">
            <div class="image-stage">
                {detail.prev.map(|p| view! {
                    <a class="stage-nav stage-prev" href=format!("/gallery/image/{p}") title="Vorheriges (←)"><Icon name="chevron-left" size=28/></a>
                })}
                <img src=img.preview_url() alt=display_title.clone() on:click=move |_| lightbox.set(Some(0))/>
                {detail.next.map(|n| view! {
                    <a class="stage-nav stage-next" href=format!("/gallery/image/{n}") title="Nächstes (→)"><Icon name="chevron-right" size=28/></a>
                })}
            </div>
            <aside class="image-side card">
                <Show when=move || editing.get() fallback={
                    let img = img.clone();
                    let display_title = display_title.clone();
                    move || view! {
                        <h1 class="image-title">{display_title.clone()}</h1>
                        {(!img.description.is_empty()).then(|| view! { <p class="image-description">{img.description.clone()}</p> })}
                    }
                }>
                    <form class="form" on:submit=save>
                        <label class="field"><span>"Titel"</span><input type="text" maxlength="120" bind:value=title/></label>
                        <label class="field"><span>"Beschreibung"</span><textarea rows="4" maxlength="5000" bind:value=description></textarea></label>
                        <label class="field">
                            <span>"Album"</span>
                            <select prop:value=move || target_album.get() on:change=move |ev| target_album.set(event_target_value(&ev))>
                                <For each=move || albums.get() key=|a| a.id children=|a| view! {
                                    <option value=a.id.to_string()>{a.title.clone()}</option>
                                }/>
                            </select>
                        </label>
                        <div class="form-actions">
                            <button type="button" class="btn" on:click=move |_| editing.set(false)>"Abbrechen"</button>
                            <button type="submit" class="btn btn-primary" disabled=move || saving.get()>"Speichern"</button>
                        </div>
                    </form>
                </Show>
                <div class="row gap-sm">
                    <Avatar user=img.owner.clone() size=28/>
                    <div>
                        <a class="user-link" href=img.owner.profile_url()>{img.owner.display_name.clone()}</a>
                        <div class="muted small">"hochgeladen " <TimeAgo ts=img.created_at/></div>
                    </div>
                </div>
                <dl class="exif">
                    {img.taken_at.map(|t| view! { <dt>"Aufgenommen"</dt><dd>{t.format("%d.%m.%Y, %H:%M").to_string()}</dd> })}
                    {img.exif.iter().map(|(k, v)| view! { <dt>{k.clone()}</dt><dd>{v.clone()}</dd> }).collect_view()}
                    <dt>"Datei"</dt><dd>{format!("{dims}{}", format_bytes(img.size_bytes))}</dd>
                    <dt>"Aufrufe"</dt><dd>{img.view_count}</dd>
                </dl>
                <ReactionBar subject_type=SubjectType::GalleryImage subject_id=id reactions=detail.reactions.clone()/>
                <div class="row gap-sm wrap">
                    <a class="btn btn-sm" href=img.download_url() rel="external"><Icon name="download" size=14/>"Original"</a>
                    <button class="btn btn-sm btn-ghost" on:click=move |_| { copy_to_clipboard(&format!("{}{}", origin(), share_url)); toasts.success("Link kopiert."); }>
                        <Icon name="share" size=14/>"Teilen"
                    </button>
                    <ReportButton subject_type=SubjectType::GalleryImage subject_id=id compact=true/>
                </div>
                {detail.can_edit.then(|| view! {
                    <div class="row gap-sm wrap">
                        <button class="btn btn-sm" on:click=start_edit><Icon name="edit" size=14/>"Bearbeiten"</button>
                        {album.can_edit.then(|| view! { <button class="btn btn-sm" on:click=set_cover><Icon name="star" size=14/>"Als Titelbild"</button> })}
                        <button class="btn btn-sm btn-danger-soft" on:click=remove><Icon name="trash" size=14/>"Löschen"</button>
                    </div>
                })}
            </aside>
        </div>
        <CommentsSection subject_type=SubjectType::GalleryImage subject_id=id/>
        <Lightbox images=single index=lightbox/>
    }
}
