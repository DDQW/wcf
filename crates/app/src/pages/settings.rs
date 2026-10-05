use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_query_map;
use shared::user::{NotifyPrefs, ProfileUpdate, SettingsView};
use shared::Theme;

use crate::api::users::{
    change_email, change_password, my_settings, set_notify_prefs, set_theme, update_profile,
};
use crate::components::{ErrorBox, Icon, MarkdownEditor, PageHeader, RequireLogin, Spinner};
use crate::state::{use_session, use_toasts};

#[component]
pub fn SettingsPage() -> impl IntoView {
    view! {
        <Title text="Einstellungen"/>
        <RequireLogin>
            <SettingsInner/>
        </RequireLogin>
    }
}

#[component]
fn SettingsInner() -> impl IntoView {
    let query = use_query_map();
    let tab = move || query.read().get("tab").unwrap_or_else(|| "profile".into());
    let data = Resource::new(|| (), |_| my_settings());
    let tab_link = move |id: &'static str, label: &'static str, icon: &'static str| {
        view! {
            <a class="tab" class:active=move || tab() == id href=format!("/settings?tab={id}")>
                <Icon name=icon size=16/>{label}
            </a>
        }
    };
    view! {
        <PageHeader title="Einstellungen"/>
        <div class="tabs">
            {tab_link("profile", "Profil", "user")}
            {tab_link("account", "Konto", "lock")}
            {tab_link("notifications", "Benachrichtigungen", "bell")}
            {tab_link("appearance", "Darstellung", "sun")}
        </div>
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(s) => match tab().as_str() {
                        "account" => view! { <AccountTab s=s/> }.into_any(),
                        "notifications" => view! { <NotifyTab prefs=s.notify_prefs.clone()/> }.into_any(),
                        "appearance" => view! { <AppearanceTab current=s.theme/> }.into_any(),
                        _ => view! { <ProfileTab s=s/> }.into_any(),
                    },
                }
            })}
        </Suspense>
    }
}

#[component]
fn ProfileTab(s: SettingsView) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let display_name = RwSignal::new(s.display_name.clone());
    let bio = RwSignal::new(s.bio_md.clone());
    let location = RwSignal::new(s.location.clone());
    let website = RwSignal::new(s.website.clone());
    let busy = RwSignal::new(false);
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        let input = ProfileUpdate {
            display_name: display_name.get_untracked(),
            bio_md: bio.get_untracked(),
            location: location.get_untracked(),
            website: website.get_untracked(),
        };
        leptos::task::spawn_local(async move {
            let name = input.display_name.trim().to_string();
            match update_profile(input).await {
                Ok(()) => {
                    toasts.success("Profil gespeichert.");
                    session.user.update(|u| {
                        if let Some(u) = u {
                            u.display_name = name;
                        }
                    });
                }
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    };
    view! {
        <div class="settings-grid">
            {crate::pages::settings_ext::avatar_section(s.avatar_url.clone())}
            <form class="card stack" on:submit=save>
                <h2>"Profil"</h2>
                <label class="field">
                    <span>"Anzeigename"</span>
                    <input type="text" maxlength="50" required bind:value=display_name/>
                </label>
                <div class="field">
                    <span>"Über mich"</span>
                    <MarkdownEditor value=bio rows=6 placeholder="Erzähl etwas über dich …"/>
                </div>
                <div class="row gap wrap">
                    <label class="field grow">
                        <span>"Ort"</span>
                        <input type="text" maxlength="100" bind:value=location/>
                    </label>
                    <label class="field grow">
                        <span>"Website"</span>
                        <input type="url" placeholder="https://" bind:value=website/>
                    </label>
                </div>
                <div class="row end">
                    <button class="btn btn-primary" disabled=move || busy.get()><Icon name="check" size=16/>"Speichern"</button>
                </div>
            </form>
        </div>
    }
}

#[component]
fn AccountTab(s: SettingsView) -> impl IntoView {
    let toasts = use_toasts();
    let email = RwSignal::new(s.email.clone());
    let email_pw = RwSignal::new(String::new());
    let current = RwSignal::new(String::new());
    let new_pw = RwSignal::new(String::new());
    let new_pw2 = RwSignal::new(String::new());

    let save_email = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        leptos::task::spawn_local(async move {
            match change_email(email_pw.get_untracked(), email.get_untracked()).await {
                Ok(()) => {
                    toasts.success("E-Mail-Adresse geändert.");
                    email_pw.set(String::new());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let save_pw = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if new_pw.get_untracked() != new_pw2.get_untracked() {
            toasts.error("Die neuen Passwörter stimmen nicht überein.");
            return;
        }
        leptos::task::spawn_local(async move {
            match change_password(current.get_untracked(), new_pw.get_untracked()).await {
                Ok(()) => {
                    toasts.success("Passwort geändert.");
                    current.set(String::new());
                    new_pw.set(String::new());
                    new_pw2.set(String::new());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <div class="settings-grid">
            <form class="card stack" on:submit=save_email>
                <h2>"E-Mail-Adresse"</h2>
                <p class="muted small">"Benutzername: @" {s.username.clone()} " (kann nicht geändert werden)"</p>
                <label class="field"><span>"E-Mail"</span><input type="email" required bind:value=email/></label>
                <label class="field"><span>"Aktuelles Passwort"</span><input type="password" required autocomplete="current-password" bind:value=email_pw/></label>
                <div class="row end"><button class="btn btn-primary">"E-Mail ändern"</button></div>
            </form>
            <form class="card stack" on:submit=save_pw>
                <h2>"Passwort"</h2>
                <label class="field"><span>"Aktuelles Passwort"</span><input type="password" required autocomplete="current-password" bind:value=current/></label>
                <label class="field"><span>"Neues Passwort"</span><input type="password" required minlength="8" autocomplete="new-password" bind:value=new_pw/></label>
                <label class="field"><span>"Neues Passwort wiederholen"</span><input type="password" required minlength="8" autocomplete="new-password" bind:value=new_pw2/></label>
                <div class="row end"><button class="btn btn-primary">"Passwort ändern"</button></div>
            </form>
        </div>
    }
}

#[component]
fn NotifyTab(prefs: NotifyPrefs) -> impl IntoView {
    let toasts = use_toasts();
    let prefs = RwSignal::new(prefs);
    let save = move || {
        let p = prefs.get_untracked();
        leptos::task::spawn_local(async move {
            if let Err(e) = set_notify_prefs(p).await {
                toasts.error(e.to_string());
            }
        });
    };
    let toggle = move |label: &'static str, hint: &'static str, get: fn(&NotifyPrefs) -> bool, set: fn(&mut NotifyPrefs, bool)| {
        view! {
            <label class="toggle-row">
                <div>
                    <div>{label}</div>
                    <div class="muted small">{hint}</div>
                </div>
                <input type="checkbox" class="switch"
                    prop:checked=move || prefs.with(get)
                    on:change=move |ev| {
                        let on = event_target_checked(&ev);
                        prefs.update(|p| set(p, on));
                        save();
                    }/>
            </label>
        }
    };
    view! {
        <div class="card stack narrow">
            <h2>"Benachrichtigungen"</h2>
            <p class="muted small">"Änderungen werden sofort gespeichert."</p>
            {toggle("Antworten", "Jemand antwortet auf deine Beiträge oder Themen", |p| p.replies, |p, v| p.replies = v)}
            {toggle("Erwähnungen", "Jemand erwähnt dich mit @name", |p| p.mentions, |p, v| p.mentions = v)}
            {toggle("Reaktionen", "Jemand reagiert auf deine Inhalte", |p| p.reactions, |p, v| p.reactions = v)}
            {toggle("Kommentare", "Neue Kommentare zu deinen Blogposts und Bildern", |p| p.comments, |p, v| p.comments = v)}
            {toggle("Abonnements", "Neues in beobachteten Themen und abonnierten Blogs", |p| p.subscriptions, |p, v| p.subscriptions = v)}
            {toggle("Direktnachrichten", "Neue private Chatnachrichten", |p| p.direct_messages, |p, v| p.direct_messages = v)}
            {toggle("Anrufe", "Verpasste Anrufe", |p| p.calls, |p, v| p.calls = v)}
            {toggle("Moderation", "Ergebnisse deiner Meldungen und Moderationshinweise", |p| p.moderation, |p, v| p.moderation = v)}
        </div>
    }
}

#[component]
fn AppearanceTab(current: Theme) -> impl IntoView {
    let toasts = use_toasts();
    let theme = expect_context::<RwSignal<Theme>>();
    let session = use_session();
    let selected = RwSignal::new(current);
    let pick = move |t: Theme| {
        selected.set(t);
        theme.set(t);
        crate::app::store_local_theme(t);
        session.user.update(|u| {
            if let Some(u) = u {
                u.theme = t;
            }
        });
        leptos::task::spawn_local(async move {
            if let Err(e) = set_theme(t).await {
                toasts.error(e.to_string());
            }
        });
    };
    view! {
        <div class="card stack narrow">
            <h2>"Darstellung"</h2>
            <div class="theme-options">
                {[Theme::System, Theme::Light, Theme::Dark].into_iter().map(|t| view! {
                    <button class="theme-option" class:active=move || selected.get() == t on:click=move |_| pick(t)>
                        <Icon name=match t { Theme::System => "monitor", Theme::Light => "sun", Theme::Dark => "moon" } size=22/>
                        <span>{t.label()}</span>
                    </button>
                }).collect_view()}
            </div>
        </div>
    }
}
