use leptos::prelude::*;
use shared::files::{FileKind, FileView};
use shared::util::format_bytes;

use crate::components::Icon;
use crate::upload::UploadOpts;

/// One entry of the visible upload queue.
#[derive(Clone)]
pub struct UploadItem {
    pub id: u64,
    pub name: String,
    pub size: i64,
    pub progress: RwSignal<f64>,
    pub error: RwSignal<Option<String>>,
    pub done: RwSignal<bool>,
}

/// Queue of running/finished uploads, shared by an upload area and its view.
#[derive(Clone, Copy)]
pub struct UploadQueue {
    pub items: RwSignal<Vec<UploadItem>>,
    #[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
    next: RwSignal<u64>,
}

impl UploadQueue {
    pub fn new() -> Self {
        Self {
            items: RwSignal::new(Vec::new()),
            next: RwSignal::new(1),
        }
    }

    pub fn clear_finished(&self) {
        self.items
            .update(|v| v.retain(|i| !i.done.get_untracked() && i.error.get_untracked().is_none()));
    }

    pub fn busy_untracked(&self) -> bool {
        self.items.with_untracked(|v| {
            v.iter()
                .any(|i| !i.done.get_untracked() && i.error.get_untracked().is_none())
        })
    }

    pub fn busy(&self) -> bool {
        self.items
            .with(|v| v.iter().any(|i| !i.done.get() && i.error.get().is_none()))
    }

    /// Uploads files one after another; `on_file` runs for each stored file.
    #[cfg(feature = "hydrate")]
    pub fn start(&self, files: Vec<web_sys::File>, opts: UploadOpts, on_file: Callback<FileView>) {
        let mut jobs = Vec::new();
        for f in files {
            let id = self.next.get_untracked();
            self.next.set(id + 1);
            let item = UploadItem {
                id,
                name: f.name(),
                size: f.size() as i64,
                progress: RwSignal::new(0.0),
                error: RwSignal::new(None),
                done: RwSignal::new(false),
            };
            self.items.update(|v| v.push(item.clone()));
            jobs.push((f, item));
        }
        leptos::task::spawn_local(async move {
            for (file, item) in jobs {
                let progress = item.progress;
                match crate::upload::upload_file(file, opts.clone(), move |p| progress.set(p)).await {
                    Ok(res) => {
                        if let Some(e) = res.errors.first() {
                            item.error.set(Some(e.clone()));
                        } else {
                            item.progress.set(1.0);
                            item.done.set(true);
                        }
                        for f in res.files {
                            on_file.run(f);
                        }
                    }
                    Err(e) => item.error.set(Some(e)),
                }
            }
        });
    }

    #[cfg(not(feature = "hydrate"))]
    pub fn start(&self, _files: Vec<()>, _opts: UploadOpts, _on_file: Callback<FileView>) {}
}

impl Default for UploadQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[component]
pub fn UploadQueueView(queue: UploadQueue) -> impl IntoView {
    view! {
        <Show when=move || !queue.items.with(|v| v.is_empty())>
            <div class="upload-queue card">
                <div class="row gap">
                    <strong class="grow">"Uploads"</strong>
                    <button class="link-btn small" on:click=move |_| queue.clear_finished()>"Erledigte ausblenden"</button>
                </div>
                <ul>
                    <For each=move || queue.items.get() key=|i| i.id children=|i| {
                        let progress = i.progress;
                        let error = i.error;
                        let done = i.done;
                        view! {
                            <li class="upload-item" class:failed=move || error.get().is_some()>
                                <div class="row gap-sm">
                                    {move || view! { <Icon name=if error.get().is_some() { "alert" } else if done.get() { "check-circle" } else { "upload" } size=15/> }}
                                    <span class="grow ellipsis">{i.name.clone()}</span>
                                    <span class="muted small">{format_bytes(i.size)}</span>
                                </div>
                                {move || match error.get() {
                                    Some(e) => view! { <div class="error small">{e}</div> }.into_any(),
                                    None => view! {
                                        <div class="progress"><div class="progress-bar" style=move || format!("width:{:.0}%", progress.get() * 100.0)></div></div>
                                    }.into_any(),
                                }}
                            </li>
                        }
                    }/>
                </ul>
            </div>
        </Show>
    }
}

/// Thumbnail image or a type icon.
#[component]
pub fn FileThumb(file: FileView, #[prop(default = 48)] size: u32) -> impl IntoView {
    let kind = file.kind();
    match file.thumb_url() {
        Some(url) => view! {
            <img class="file-thumb" src=url alt="" loading="lazy" style=format!("width:{size}px;height:{size}px")/>
        }
        .into_any(),
        None => view! {
            <span class=format!("file-icon kind-{}", kind_class(kind)) style=format!("width:{size}px;height:{size}px")>
                <Icon name=kind.icon() size=size / 2/>
            </span>
        }
        .into_any(),
    }
}

pub fn kind_class(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Image => "image",
        FileKind::Video => "video",
        FileKind::Audio => "audio",
        FileKind::Pdf => "pdf",
        FileKind::Text => "text",
        FileKind::Archive => "archive",
        FileKind::Document => "doc",
        FileKind::Other => "other",
    }
}
