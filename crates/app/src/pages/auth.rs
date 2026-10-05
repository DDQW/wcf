use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::api::auth::{login, register, request_password_reset, reset_password};
use crate::components::Icon;
use crate::state::{use_session, use_toasts};

/// Where to go after login: the `next` query parameter (local paths only).
fn next_target() -> String {
    let query = use_query_map();
    query
        .with_untracked(|q| q.get("next"))
        .filter(|n| n.starts_with('/') && !n.starts_with("//"))
        .unwrap_or_else(|| "/".to_string())
}

#[component]
pub fn LoginPage() -> impl IntoView {
    let session = use_session();
    let navigate = use_navigate();
    let ident = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let target = next_target();

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let navigate = navigate.clone();
        let target = target.clone();
        leptos::task::spawn_local(async move {
            match login(ident.get_untracked(), password.get_untracked()).await {
                Ok(b) => {
                    session.apply(b);
                    navigate(&target, Default::default());
                }
                Err(e) => error.set(Some(e.to_string())),
            }
            busy.set(false);
        });
    };

    view! {
        <Title text="Anmelden"/>
        <div class="auth-card card">
            <h1>"Anmelden"</h1>
            <form class="stack" on:submit=submit>
                <label class="field">
                    <span>"Benutzername oder E-Mail"</span>
                    <input type="text" autocomplete="username" required bind:value=ident/>
                </label>
                <label class="field">
                    <span>"Passwort"</span>
                    <input type="password" autocomplete="current-password" required bind:value=password/>
                </label>
                {move || error.get().map(|e| view! { <div class="alert alert-error"><Icon name="alert"/>{e}</div> })}
                <button class="btn btn-primary btn-block" type="submit" disabled=move || busy.get()>
                    <Icon name="log-in" size=17/>"Anmelden"
                </button>
            </form>
            <div class="auth-links">
                <a href="/forgot-password">"Passwort vergessen?"</a>
                <a href="/register">"Noch kein Konto? Registrieren"</a>
            </div>
        </div>
    }
}

#[component]
pub fn RegisterPage() -> impl IntoView {
    let session = use_session();
    let navigate = use_navigate();
    let username = RwSignal::new(String::new());
    let display_name = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let password2 = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let open = move || session.site.with(|s| s.registration_open);

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if let Err(e) = shared::util::validate_username(username.get_untracked().trim()) {
            error.set(Some(e.to_string()));
            return;
        }
        if password.get_untracked() != password2.get_untracked() {
            error.set(Some("Die Passwörter stimmen nicht überein.".into()));
            return;
        }
        busy.set(true);
        error.set(None);
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match register(
                username.get_untracked(),
                email.get_untracked(),
                password.get_untracked(),
                display_name.get_untracked(),
            )
            .await
            {
                Ok(b) => {
                    session.apply(b);
                    navigate("/", Default::default());
                }
                Err(e) => error.set(Some(e.to_string())),
            }
            busy.set(false);
        });
    };

    view! {
        <Title text="Registrieren"/>
        <div class="auth-card card">
            <h1>"Konto erstellen"</h1>
            <Show when=open fallback=|| view! {
                <div class="alert"><Icon name="lock"/>"Die Registrierung ist derzeit geschlossen."</div>
            }>
                <form class="stack" on:submit=submit.clone()>
                    <label class="field">
                        <span>"Benutzername"</span>
                        <input type="text" autocomplete="username" required minlength="3" maxlength="32" bind:value=username/>
                        <small class="muted">"3–32 Zeichen: Buchstaben, Ziffern, _ - ."</small>
                    </label>
                    <label class="field">
                        <span>"Anzeigename (optional)"</span>
                        <input type="text" maxlength="50" bind:value=display_name/>
                    </label>
                    <label class="field">
                        <span>"E-Mail"</span>
                        <input type="email" autocomplete="email" required bind:value=email/>
                    </label>
                    <label class="field">
                        <span>"Passwort"</span>
                        <input type="password" autocomplete="new-password" required minlength="8" bind:value=password/>
                    </label>
                    <label class="field">
                        <span>"Passwort wiederholen"</span>
                        <input type="password" autocomplete="new-password" required minlength="8" bind:value=password2/>
                    </label>
                    {move || error.get().map(|e| view! { <div class="alert alert-error"><Icon name="alert"/>{e}</div> })}
                    <button class="btn btn-primary btn-block" type="submit" disabled=move || busy.get()>
                        <Icon name="user-plus" size=17/>"Registrieren"
                    </button>
                </form>
            </Show>
            <div class="auth-links">
                <a href="/login">"Schon registriert? Anmelden"</a>
            </div>
        </div>
    }
}

#[component]
pub fn ForgotPasswordPage() -> impl IntoView {
    let session = use_session();
    let email = RwSignal::new(String::new());
    let sent = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let by_mail = move || session.site.with(|s| s.password_reset_by_mail);

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        leptos::task::spawn_local(async move {
            match request_password_reset(email.get_untracked()).await {
                Ok(()) => sent.set(true),
                Err(e) => error.set(Some(e.to_string())),
            }
            busy.set(false);
        });
    };

    view! {
        <Title text="Passwort vergessen"/>
        <div class="auth-card card">
            <h1>"Passwort vergessen"</h1>
            <Show when=by_mail fallback=|| view! {
                <p>"Auf dieser Seite ist kein E-Mail-Versand eingerichtet. Bitte wende dich an einen Admin – "
                   "dieser kann dir einen Link zum Zurücksetzen erstellen."</p>
            }>
                <Show when=move || !sent.get() fallback=|| view! {
                    <div class="alert alert-success"><Icon name="check-circle"/>
                        "Falls ein Konto mit dieser Adresse existiert, haben wir dir eine E-Mail geschickt."
                    </div>
                }>
                    <form class="stack" on:submit=submit>
                        <p class="muted">"Wir schicken dir einen Link, mit dem du ein neues Passwort setzen kannst."</p>
                        <label class="field">
                            <span>"E-Mail"</span>
                            <input type="email" required bind:value=email/>
                        </label>
                        {move || error.get().map(|e| view! { <div class="alert alert-error">{e}</div> })}
                        <button class="btn btn-primary btn-block" disabled=move || busy.get()>"Link anfordern"</button>
                    </form>
                </Show>
            </Show>
            <div class="auth-links"><a href="/login">"Zurück zur Anmeldung"</a></div>
        </div>
    }
}

#[component]
pub fn ResetPasswordPage() -> impl IntoView {
    let query = use_query_map();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let password = RwSignal::new(String::new());
    let password2 = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if password.get_untracked() != password2.get_untracked() {
            error.set(Some("Die Passwörter stimmen nicht überein.".into()));
            return;
        }
        let token = query.with_untracked(|q| q.get("token")).unwrap_or_default();
        busy.set(true);
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match reset_password(token, password.get_untracked()).await {
                Ok(()) => {
                    toasts.success("Passwort geändert – du kannst dich jetzt anmelden.");
                    navigate("/login", Default::default());
                }
                Err(e) => error.set(Some(e.to_string())),
            }
            busy.set(false);
        });
    };

    view! {
        <Title text="Neues Passwort"/>
        <div class="auth-card card">
            <h1>"Neues Passwort festlegen"</h1>
            <form class="stack" on:submit=submit>
                <label class="field">
                    <span>"Neues Passwort"</span>
                    <input type="password" autocomplete="new-password" required minlength="8" bind:value=password/>
                </label>
                <label class="field">
                    <span>"Wiederholen"</span>
                    <input type="password" autocomplete="new-password" required minlength="8" bind:value=password2/>
                </label>
                {move || error.get().map(|e| view! { <div class="alert alert-error">{e}</div> })}
                <button class="btn btn-primary btn-block" disabled=move || busy.get()>"Speichern"</button>
            </form>
        </div>
    }
}
