use leptos::prelude::*;
use shared::SiteSettings;

use crate::api::admin::{get_site_settings, save_site_settings};
use crate::components::{ErrorBox, Icon, MarkdownEditor, Spinner};
use crate::state::{use_session, use_toasts};

#[component]
pub fn AdminSettings() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_site_settings());
    view! {
        <h1>"Einstellungen"</h1>
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(s) => view! { <SettingsForm s=s/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn SettingsForm(s: SiteSettings) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let settings = RwSignal::new(s.clone());
    let welcome = RwSignal::new(s.welcome_md.clone());
    let busy = RwSignal::new(false);

    let text = move |label: &'static str, get: fn(&SiteSettings) -> String, set: fn(&mut SiteSettings, String)| view! {
        <label class="field">
            <span>{label}</span>
            <input type="text" prop:value=move || settings.with(get)
                on:input=move |ev| settings.update(|s| set(s, event_target_value(&ev)))/>
        </label>
    };
    let number = move |label: &'static str, get: fn(&SiteSettings) -> i64, set: fn(&mut SiteSettings, i64)| view! {
        <label class="field">
            <span>{label}</span>
            <input type="number" min="0" prop:value=move || settings.with(get).to_string()
                on:input=move |ev| { if let Ok(v) = event_target_value(&ev).parse() { settings.update(|s| set(s, v)) } }/>
        </label>
    };
    let check = move |label: &'static str, get: fn(&SiteSettings) -> bool, set: fn(&mut SiteSettings, bool)| view! {
        <label class="toggle-row">
            <span>{label}</span>
            <input type="checkbox" class="switch" prop:checked=move || settings.with(get)
                on:change=move |ev| settings.update(|s| set(s, event_target_checked(&ev)))/>
        </label>
    };

    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        let mut s = settings.get_untracked();
        s.welcome_md = welcome.get_untracked();
        leptos::task::spawn_local(async move {
            match save_site_settings(s).await {
                Ok(saved) => {
                    toasts.success("Einstellungen gespeichert.");
                    settings.set(saved);
                    if let Ok(b) = crate::api::auth::load_bootstrap().await {
                        session.apply(b);
                    }
                }
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    };

    view! {
        <form class="stack" on:submit=save>
            <section class="card stack">
                <h2>"Allgemein"</h2>
                {text("Name der Seite", |s| s.site_name.clone(), |s, v| s.site_name = v)}
                {text("Beschreibung", |s| s.site_description.clone(), |s, v| s.site_description = v)}
                <div class="field">
                    <span>"Willkommenstext (Startseite)"</span>
                    <MarkdownEditor value=welcome rows=5/>
                </div>
                {check("Registrierung offen", |s| s.registration_open, |s, v| s.registration_open = v)}
            </section>
            <section class="card stack">
                <h2>"Module"</h2>
                {check("Forum", |s| s.modules.forum, |s, v| s.modules.forum = v)}
                {check("Chat", |s| s.modules.chat, |s, v| s.modules.chat = v)}
                {check("Blogs", |s| s.modules.blogs, |s, v| s.modules.blogs = v)}
                {check("Galerie", |s| s.modules.gallery, |s, v| s.modules.gallery = v)}
                {check("Dateien", |s| s.modules.files, |s, v| s.modules.files = v)}
                {check("Video-/Audiocalls", |s| s.modules.calls, |s, v| s.modules.calls = v)}
                {check("Watch & Kino", |s| s.modules.watch, |s, v| s.modules.watch = v)}
                {check("Seiten & Inhalte (CMS)", |s| s.modules.cms, |s, v| s.modules.cms = v)}
            </section>
            <section class="card stack">
                <h2>"Uploads & Rechte"</h2>
                {number("Maximale Dateigröße pro Upload (MB)", |s| s.max_upload_mb, |s, v| s.max_upload_mb = v)}
                {number("Standard-Speicherkontingent pro Benutzer (MB, 0 = unbegrenzt)", |s| s.default_quota_mb, |s, v| s.default_quota_mb = v)}
                {check("Mitglieder dürfen Chat-Kanäle erstellen", |s| s.members_create_channels, |s, v| s.members_create_channels = v)}
                {check("Mitglieder dürfen Watch-Räume erstellen", |s| s.members_create_watch_rooms, |s, v| s.members_create_watch_rooms = v)}
                {check("Mitglieder dürfen Call-Räume erstellen", |s| s.members_create_call_rooms, |s, v| s.members_create_call_rooms = v)}
            </section>
            <div class="row end">
                <button class="btn btn-primary" disabled=move || busy.get()><Icon name="check" size=16/>"Speichern"</button>
            </div>
        </form>
    }
}
