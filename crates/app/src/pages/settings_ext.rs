//! Settings sections contributed by optional modules (avatar upload).

use leptos::prelude::*;

use crate::api::files::set_avatar;
use crate::components::{Avatar, Icon};
use crate::state::{use_session, use_toasts};

pub fn avatar_section(avatar_url: Option<String>) -> impl IntoView {
    view! { <AvatarSection current=avatar_url/> }
}

#[component]
fn AvatarSection(current: Option<String>) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let busy = RwSignal::new(false);
    let _ = current;

    let apply = move |url: Option<String>| {
        session.user.update(|u| {
            if let Some(u) = u {
                u.avatar_url = url;
            }
        });
    };
    let on_pick = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            let files = crate::upload::files_from_input(&ev);
            let Some(file) = files.into_iter().next() else { return };
            busy.set(true);
            leptos::task::spawn_local(async move {
                let opts = crate::upload::UploadOpts {
                    purpose: shared::files::FilePurpose::Avatar,
                    visibility: Some(shared::files::Visibility::Public),
                    ..Default::default()
                };
                match crate::upload::upload_file(file, opts, |_| {}).await {
                    Ok(res) => match (res.files.first(), res.errors.first()) {
                        (Some(f), _) if f.has_thumb => {
                            let id = f.id;
                            match set_avatar(Some(id)).await {
                                Ok(()) => {
                                    apply(Some(format!("/files/{id}/thumb")));
                                    toasts.success("Avatar gespeichert.");
                                }
                                Err(e) => toasts.error(e.to_string()),
                            }
                        }
                        (Some(_), _) => toasts.error("Bitte lade ein Bild (JPG, PNG, GIF, WebP) hoch."),
                        (None, Some(e)) => toasts.error(e.clone()),
                        _ => {}
                    },
                    Err(e) => toasts.error(e),
                }
                busy.set(false);
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = (ev, apply, busy, toasts);
    };
    let remove = move |_| {
        leptos::task::spawn_local(async move {
            if set_avatar(None).await.is_ok() {
                apply(None);
            }
        });
    };
    view! {
        <section class="card stack avatar-settings">
            <h2>"Avatar"</h2>
            <div class="row gap">
                {move || session.user.get().map(|u| view! { <Avatar user=u.summary() size=88/> })}
                <div class="stack-sm">
                    <label class="btn">
                        <Icon name="upload" size=16/>{move || if busy.get() { "Lade hoch …" } else { "Bild hochladen" }}
                        <input type="file" accept="image/*" class="hidden" on:change=on_pick/>
                    </label>
                    <Show when=move || session.user.with(|u| u.as_ref().is_some_and(|u| u.avatar_url.is_some()))>
                        <button class="btn btn-ghost" on:click=remove><Icon name="trash" size=16/>"Entfernen"</button>
                    </Show>
                    <p class="muted small">"Quadratische Bilder sehen am besten aus."</p>
                </div>
            </div>
        </section>
    }
}
