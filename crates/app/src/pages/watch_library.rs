//! Media library admin (port of HQTube's Admin → Library + Emotes): import
//! from the server's incoming folder, delivery mode, track selection for
//! HLS packaging, progress, rename/delete, storage usage, chat emotes.

use leptos::prelude::*;
use leptos_meta::Title;
use shared::util::format_bytes;
use shared::watch::{fmt_timecode, ImportMode, MediaInfo, MediaStatus, PackageSelection, PlayKind, ProbeKind, ProbeTrack};
use uuid::Uuid;

use crate::api::watch::*;
use crate::components::common::confirm;
use crate::components::{EmptyState, Icon, Modal, RequireLogin, Spinner};
use crate::pages::watch::local;
use crate::state::{use_session, use_toasts};

#[component]
pub fn WatchLibraryPage() -> impl IntoView {
    let session = use_session();
    view! {
        <Title text="Mediathek"/>
        <RequireLogin>
            <Show when=move || session.is_admin() fallback=|| view! { <div class="card"><p class="error">"Die Mediathek verwalten nur Administratoren."</p></div> }>
                <Library/>
            </Show>
        </RequireLogin>
    }
}

#[component]
fn Library() -> impl IntoView {
    let toasts = use_toasts();
    let version = RwSignal::new(0u32);
    let overview = LocalResource::new(move || {
        version.track();
        async move { watch_library().await }
    });
    let import_version = RwSignal::new(0u32);
    let files = LocalResource::new(move || {
        import_version.track();
        async move { watch_import_files().await.unwrap_or_default() }
    });
    // Poll while something is being prepared.
    let busy = move || {
        local(overview)
            .and_then(|r| r.ok())
            .is_some_and(|o| o.media.iter().any(|m| matches!(m.status, MediaStatus::Probing | MediaStatus::Transcoding)))
    };
    #[cfg(feature = "hydrate")]
    {
        let handle = set_interval_with_handle(
            move || {
                if busy() {
                    version.update(|v| *v += 1);
                }
            },
            std::time::Duration::from_secs(3),
        )
        .ok();
        on_cleanup(move || {
            if let Some(h) = handle {
                h.clear();
            }
        });
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = busy;
    let package_target = RwSignal::new(None::<MediaInfo>);
    let package_open = RwSignal::new(false);
    Effect::new(move |_| {
        if !package_open.get() {
            package_target.set(None);
        }
    });
    let reload = Callback::new(move |_| {
        version.update(|v| *v += 1);
        import_version.update(|v| *v += 1);
    });

    let path = RwSignal::new(String::new());
    let path_title = RwSignal::new(String::new());
    let path_mode = RwSignal::new(ImportMode::Auto);
    let import_path = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let (t, p, m) = (path_title.get_untracked(), path.get_untracked(), path_mode.get_untracked());
        leptos::task::spawn_local(async move {
            match import_watch_media(t, p, m).await {
                Ok(_) => {
                    path.set(String::new());
                    path_title.set(String::new());
                    reload.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    view! {
        <div class="page-header">
            <div>
                <h1>"Mediathek"</h1>
                <p class="muted">"Filme für Kinoräume und Vorstellungen – unverändert abgespielt oder verlustfrei als HLS mit umschaltbaren Tonspuren verpackt."</p>
            </div>
            <div class="page-actions"><a class="btn" href="/watch"><Icon name="tv" size=16/>"Zum Kino"</a></div>
        </div>
        {move || local(overview).map(|r| match r {
            Err(e) => view! { <crate::components::ErrorBox error=e/> }.into_any(),
            Ok(o) => view! {
                <div class="stat-row">
                    <div class="stat"><span class="stat-value">{o.media.len()}</span><span class="stat-label">"Filme"</span></div>
                    <div class="stat">
                        <span class="stat-value">{format_bytes(o.used_bytes)}</span>
                        <span class="stat-label">{match o.cap_bytes { Some(c) => format!("von {} belegt", format_bytes(c)), None => "belegt (ohne Limit)".into() }}</span>
                    </div>
                    <div class="stat">
                        <span class="stat-value">{if o.ffmpeg { "✓" } else { "✗" }}</span>
                        <span class="stat-label">"ffmpeg"</span>
                    </div>
                </div>
                {(!o.ffmpeg).then(|| view! { <div class="alert">"ffmpeg wurde nicht gefunden – Import und Verpacken sind erst möglich, wenn ffmpeg installiert ist (oder FFMPEG_PATH gesetzt wird)."</div> })}
                <section class="card">
                    <div class="card-head"><h2><Icon name="film"/>"Filme"</h2></div>
                    {if o.media.is_empty() {
                        view! { <EmptyState icon="film" title="Noch keine Filme"/> }.into_any()
                    } else {
                        view! {
                            <ul class="plain-list media-list">
                                {o.media.into_iter().map(|m| view! { <MediaRow media=m reload=reload package_target=package_target package_open=package_open/> }).collect_view()}
                            </ul>
                        }.into_any()
                    }}
                </section>
                <section class="card">
                    <div class="card-head">
                        <h2><Icon name="inbox"/>"Import-Ordner"</h2>
                        <button class="btn btn-sm btn-ghost" on:click=move |_| import_version.update(|v| *v += 1)><Icon name="refresh" size=14/>"Neu einlesen"</button>
                    </div>
                    <p class="muted small mono">{o.import_dir.clone()}</p>
                    <p class="muted small">"Videodateien in diesem Ordner (auch Unterordner und Verknüpfungen) erscheinen hier zum Import mit einem Klick."</p>
                    <ImportList files=files reload=reload/>
                    <h3>"Per Pfad importieren"</h3>
                    <form class="form-row" on:submit=import_path>
                        <input class="grow" type="text" required placeholder="Absoluter Pfad auf dem Server" bind:value=path/>
                        <input type="text" required placeholder="Titel" bind:value=path_title/>
                        <ModeSelect mode=path_mode/>
                        <button class="btn btn-primary" type="submit">"Importieren"</button>
                    </form>
                </section>
            }.into_any(),
        }).unwrap_or_else(|| view! { <Spinner/> }.into_any())}
        <EmoteAdmin/>
        <Modal open=package_open title="Spuren für HLS wählen" wide=true>
            {move || package_target.get().map(|m| view! { <PackageForm media=m done=Callback::new(move |_| { package_open.set(false); reload.run(()); })/> })}
        </Modal>
    }
}

#[component]
fn ModeSelect(mode: RwSignal<ImportMode>) -> impl IntoView {
    view! {
        <select title="Auslieferung" prop:value=move || match mode.get() { ImportMode::Auto => "auto", ImportMode::Direct => "direct", ImportMode::Hls => "hls" }
            on:change=move |ev| mode.set(match event_target_value(&ev).as_str() { "direct" => ImportMode::Direct, "hls" => ImportMode::Hls, _ => ImportMode::Auto })>
            <option value="auto">"Automatisch"</option>
            <option value="direct">"Direct Play (unverändert)"</option>
            <option value="hls">"HLS-Remux (mehrere Tonspuren)"</option>
        </select>
    }
}

#[component]
fn ImportList(files: LocalResource<Vec<shared::watch::ImportFile>>, reload: Callback<()>) -> impl IntoView {
    let toasts = use_toasts();
    view! {
        <ul class="plain-list import-list">
            {move || {
                let list = local(files).unwrap_or_default();
                if list.is_empty() {
                    return view! { <li class="muted small">"Keine Videodateien im Import-Ordner."</li> }.into_any();
                }
                list.into_iter().map(|f| {
                    let title = RwSignal::new(f.suggested_title.clone());
                    let mode = RwSignal::new(ImportMode::Auto);
                    let busy = RwSignal::new(false);
                    let path = f.path.clone();
                    let import = move |_| {
                        let (t, p, m) = (title.get_untracked(), path.clone(), mode.get_untracked());
                        busy.set(true);
                        leptos::task::spawn_local(async move {
                            match import_watch_media(t, p, m).await {
                                Ok(_) => { toasts.success("Importiert."); reload.run(()); }
                                Err(e) => toasts.error(e.to_string()),
                            }
                            busy.set(false);
                        });
                    };
                    view! {
                        <li class="import-row">
                            <div class="grow">
                                <div class="mono small ellipsis" title=f.rel_path.clone()>{f.rel_path.clone()}</div>
                                <div class="muted small">
                                    {format_bytes(f.size)}
                                    {f.imported_as.clone().map(|t| format!(" · bereits importiert als „{t}“"))}
                                </div>
                            </div>
                            <input type="text" bind:value=title/>
                            <ModeSelect mode=mode/>
                            <button class="btn btn-sm btn-primary" disabled=move || busy.get() on:click=import>
                                {move || if busy.get() { "Analysiere …" } else { "Importieren" }}
                            </button>
                        </li>
                    }
                }).collect_view().into_any()
            }}
        </ul>
    }
}

#[component]
fn MediaRow(
    media: MediaInfo,
    reload: Callback<()>,
    package_target: RwSignal<Option<MediaInfo>>,
    package_open: RwSignal<bool>,
) -> impl IntoView {
    let toasts = use_toasts();
    let id = media.id;
    let renaming = RwSignal::new(false);
    let title = RwSignal::new(media.title.clone());
    let can_package = media.probe.is_some();
    let status = media.status;
    let pkg = media.clone();
    let save_title = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let t = title.get_untracked();
        leptos::task::spawn_local(async move {
            match rename_watch_media(id, t).await {
                Ok(_) => {
                    renaming.set(false);
                    reload.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let remove = move |_| {
        if !confirm("Film und seine aufbereiteten Dateien löschen? Die Quelldatei bleibt erhalten.") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_watch_media(id).await {
                Ok(archived) => {
                    if archived {
                        toasts.info("Archiviert – der Film bleibt im Verlauf vergangener Vorstellungen.", None);
                    }
                    reload.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let audio = media.tracks.iter().filter(|t| t.kind == shared::watch::TrackKind::Audio).count();
    let subs = media.tracks.iter().filter(|t| t.kind == shared::watch::TrackKind::Sub).count();
    view! {
        <li class="media-row">
            <div class="media-poster">
                {match media.poster_url.clone() {
                    Some(p) => view! { <img src=p alt="" loading="lazy"/> }.into_any(),
                    None => view! { <Icon name="film" size=26/> }.into_any(),
                }}
            </div>
            <div class="grow">
                <Show when=move || renaming.get() fallback=move || view! { <strong>{move || title.get()}</strong> }>
                    <form class="row gap-sm" on:submit=save_title>
                        <input class="grow" type="text" maxlength="200" bind:value=title/>
                        <button class="btn btn-sm btn-primary" type="submit">"OK"</button>
                    </form>
                </Show>
                <div class="muted small">
                    <span class=format!("tag-chip media-{}", status.as_str())>{status.label()}</span>
                    {media.play_kind.map(|k| match k { PlayKind::Hls => " · HLS", PlayKind::Direct => " · Direct Play", PlayKind::Youtube => "" })}
                    {media.duration_ms.map(|d| format!(" · {}", fmt_timecode(d as f64)))}
                    {(audio > 0).then(|| format!(" · {audio} Ton"))}
                    {(subs > 0).then(|| format!(" · {subs} Untertitel"))}
                    {media.size_bytes.map(|s| format!(" · {}", format_bytes(s)))}
                    {media.source_name.clone().map(|n| format!(" · {n}"))}
                </div>
                {media.progress.map(|p| view! {
                    <div class="progress"><div class="progress-bar" style=format!("width:{:.0}%", p * 100.0)></div></div>
                })}
                {media.error.clone().map(|e| view! { <div class="error small">{e}</div> })}
            </div>
            <div class="row gap-sm">
                {can_package.then(|| view! {
                    <button class="btn btn-sm" class:btn-primary=status == MediaStatus::New on:click=move |_| { package_target.set(Some(pkg.clone())); package_open.set(true); }>
                        <Icon name="layers" size=14/>{if status == MediaStatus::Ready { "Neu verpacken" } else { "Verpacken" }}
                    </button>
                })}
                <button class="icon-btn" title="Umbenennen" on:click=move |_| renaming.update(|r| *r = !*r)><Icon name="edit" size=14/></button>
                <button class="icon-btn" title="Löschen" disabled=matches!(status, MediaStatus::Probing | MediaStatus::Transcoding) on:click=remove><Icon name="trash" size=14/></button>
            </div>
        </li>
    }
}

fn track_label(t: &ProbeTrack) -> String {
    let mut parts = vec![t.codec.clone()];
    if let Some(l) = &t.lang {
        parts.push(l.clone());
    }
    if let Some(c) = t.channels {
        parts.push(format!("{c} ch"));
    }
    if let Some(title) = &t.title {
        parts.push(format!("„{title}“"));
    }
    if t.is_default {
        parts.push("Standard".into());
    }
    parts.join(" · ")
}

#[component]
fn PackageForm(media: MediaInfo, done: Callback<()>) -> impl IntoView {
    let toasts = use_toasts();
    let probe = media.probe.clone().unwrap_or_default();
    let audio: Vec<ProbeTrack> = probe.iter().filter(|t| t.kind == ProbeKind::Audio).cloned().collect();
    let subs: Vec<ProbeTrack> = probe.iter().filter(|t| t.kind == ProbeKind::Sub).cloned().collect();
    let video = probe.iter().find(|t| t.kind == ProbeKind::Video).cloned();
    // Default: every audio track, every text subtitle.
    let audio_sel = RwSignal::new(audio.iter().map(|t| t.type_index).collect::<Vec<_>>());
    let sub_sel = RwSignal::new(subs.iter().filter(|t| t.text_based).map(|t| t.type_index).collect::<Vec<_>>());
    let busy = RwSignal::new(false);
    let toggle = |sel: RwSignal<Vec<i32>>, i: i32| sel.update(|v| if v.contains(&i) { v.retain(|x| *x != i) } else { v.push(i); v.sort(); });
    let id = media.id;
    let submit = move |_| {
        let selection = PackageSelection { audio: audio_sel.get_untracked(), subs: sub_sel.get_untracked() };
        busy.set(true);
        leptos::task::spawn_local(async move {
            match package_watch_media(id, selection).await {
                Ok(()) => {
                    toasts.success("Wird verpackt – der Fortschritt erscheint in der Liste.");
                    done.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    };
    view! {
        <div class="form">
            <p><strong>{media.title.clone()}</strong></p>
            {video.map(|v| view! {
                <p class="muted small">{format!("Video: {}{}", v.codec, match (v.width, v.height) { (Some(w), Some(h)) => format!(" · {w}×{h}"), _ => String::new() })}
                    {(!matches!(v.codec.as_str(), "h264" | "av1" | "vp9")).then_some(" – wird nach H.264 umkodiert (dauert länger)")}
                </p>
            })}
            <h3>"Tonspuren"</h3>
            <div class="track-list">
                {audio.into_iter().map(|t| {
                    let i = t.type_index;
                    view! {
                        <label class="check">
                            <input type="checkbox" prop:checked=move || audio_sel.with(|v| v.contains(&i)) on:change=move |_| toggle(audio_sel, i)/>
                            <span>{track_label(&t)}</span>
                        </label>
                    }
                }).collect_view()}
            </div>
            <h3>"Untertitel"</h3>
            <div class="track-list">
                {if subs.is_empty() {
                    view! { <p class="muted small">"Keine Untertitelspuren."</p> }.into_any()
                } else {
                    subs.into_iter().map(|t| {
                        let i = t.type_index;
                        let text = t.text_based;
                        view! {
                            <label class="check" class:disabled=!text>
                                <input type="checkbox" disabled=!text prop:checked=move || sub_sel.with(|v| v.contains(&i)) on:change=move |_| toggle(sub_sel, i)/>
                                <span>{track_label(&t)}{(!text).then_some(" – Bild-Untertitel, nicht umwandelbar")}</span>
                            </label>
                        }
                    }).collect_view().into_any()
                }}
            </div>
            <p class="muted small">"Browser-taugliche Codecs (H.264/AV1/VP9, AAC/Opus/FLAC/MP3) werden verlustfrei kopiert – das dauert nur Sekunden."</p>
            <div class="form-actions">
                <button class="btn btn-primary" disabled=move || busy.get() || audio_sel.with(|v| v.is_empty()) on:click=submit>"Verpacken"</button>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Emotes
// ---------------------------------------------------------------------------

#[component]
fn EmoteAdmin() -> impl IntoView {
    let toasts = use_toasts();
    let version = RwSignal::new(0u32);
    let emotes = LocalResource::new(move || {
        version.track();
        async move { watch_emotes().await.unwrap_or_default() }
    });
    let code = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let input_ref = NodeRef::<leptos::html::Input>::new();
    let upload = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        #[cfg(feature = "hydrate")]
        {
            let Some(input) = input_ref.get_untracked() else { return };
            let Some(file) = input.files().and_then(|f| f.get(0)) else {
                toasts.error("Bitte wähle ein Bild.");
                return;
            };
            let c = code.get_untracked();
            busy.set(true);
            leptos::task::spawn_local(async move {
                let opts = crate::upload::UploadOpts {
                    purpose: shared::files::FilePurpose::Watch,
                    visibility: Some(shared::files::Visibility::Public),
                    ..Default::default()
                };
                let res = crate::upload::upload_file(file, opts, |_| {}).await;
                let file_id = match res {
                    Ok(r) => match (r.files.first(), r.errors.first()) {
                        (Some(f), _) => Some(f.id),
                        (None, Some(e)) => {
                            toasts.error(e.clone());
                            None
                        }
                        _ => None,
                    },
                    Err(e) => {
                        toasts.error(e);
                        None
                    }
                };
                if let Some(fid) = file_id {
                    match create_watch_emote(c, fid).await {
                        Ok(_) => {
                            code.set(String::new());
                            version.update(|v| *v += 1);
                        }
                        Err(e) => toasts.error(e.to_string()),
                    }
                }
                busy.set(false);
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = (input_ref, busy, code);
    };
    let remove = move |id: Uuid| {
        leptos::task::spawn_local(async move {
            match delete_watch_emote(id).await {
                Ok(()) => version.update(|v| *v += 1),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <section class="card">
            <div class="card-head"><h2><Icon name="smile"/>"Emotes"</h2></div>
            <p class="muted small">"Im Raum-Chat als :code: nutzbar. PNG, GIF oder WebP bis 256 KB."</p>
            <form class="form-row" on:submit=upload>
                <input type="text" required pattern="[a-z0-9_]{2,24}" placeholder="code (a–z, 0–9, _)" bind:value=code/>
                <input type="file" accept="image/png,image/gif,image/webp" node_ref=input_ref/>
                <button class="btn btn-primary" type="submit" disabled=move || busy.get()>"Hinzufügen"</button>
            </form>
            <div class="emote-grid">
                {move || local(emotes).unwrap_or_default().into_iter().map(|e| {
                    let id = e.id;
                    view! {
                        <div class="emote-tile">
                            <img class="emote" src=e.url.clone() alt=""/>
                            <code>{format!(":{}:", e.code)}</code>
                            <button class="icon-btn" title="Löschen" on:click=move |_| remove(id)><Icon name="x" size=12/></button>
                        </div>
                    }
                }).collect_view()}
            </div>
        </section>
    }
}
