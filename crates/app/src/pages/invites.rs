//! Invite codes: admin page and the members' section in the settings.

use leptos::prelude::*;
use leptos_meta::Title;
use shared::admin::{InviteInput, InviteView};

use crate::api::invites::{create_invites, list_invites, revoke_invite};
use crate::components::common::{copy_to_clipboard, local_datetime, origin};
use crate::components::{Avatar, Icon};
use crate::pages::watch::local;
use crate::state::{use_session, use_toasts};

#[component]
pub fn AdminInvites() -> impl IntoView {
    let session = use_session();
    view! {
        <Title text="Einladungen"/>
        <h1>"Einladungen"</h1>
        <p class="muted">
            {move || if session.site.with(|s| s.invite_only) {
                "Die Registrierung ist nur mit Einladungscode möglich."
            } else {
                "Die Registrierung ist ohne Code möglich – Codes wirken erst, wenn „Registrierung nur mit Einladungscode“ in den Einstellungen aktiv ist."
            }}
        </p>
        <InviteManager/>
    }
}

/// Settings section for members (when the site allows member invites).
#[component]
pub fn InviteSection() -> impl IntoView {
    let session = use_session();
    let visible = move || session.site.with(|s| s.members_can_invite) || session.is_mod();
    view! {
        <Show when=visible>
            <section class="card stack">
                <h2>"Freunde einladen"</h2>
                <p class="muted small">"Erstelle einen Einladungslink. Er ist einmal gültig und läuft nach 30 Tagen ab."</p>
                <InviteManager/>
            </section>
        </Show>
    }
}

#[component]
fn InviteManager() -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let staff = move || session.is_mod();
    let version = RwSignal::new(0u32);
    let invites = LocalResource::new(move || {
        version.track();
        async move { list_invites().await.unwrap_or_default() }
    });
    let count = RwSignal::new("1".to_string());
    let uses = RwSignal::new("1".to_string());
    let days = RwSignal::new("30".to_string());
    let note = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let show_used = RwSignal::new(false);

    let create = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let d = days.get_untracked();
        let input = InviteInput {
            count: count.get_untracked().trim().parse().unwrap_or(1),
            max_uses: uses.get_untracked().trim().parse().unwrap_or(1),
            expires_days: d.trim().parse::<i64>().ok().filter(|d| *d > 0),
            note: note.get_untracked(),
        };
        busy.set(true);
        leptos::task::spawn_local(async move {
            match create_invites(input).await {
                Ok(created) => {
                    if let [one] = created.as_slice() {
                        copy_to_clipboard(&one.link(&origin()));
                        toasts.success("Einladungslink erstellt und kopiert.");
                    } else {
                        toasts.success(format!("{} Einladungen erstellt.", created.len()));
                    }
                    note.set(String::new());
                    version.update(|v| *v += 1);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    };
    let revoke = move |code: String| {
        leptos::task::spawn_local(async move {
            match revoke_invite(code).await {
                Ok(()) => version.update(|v| *v += 1),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    view! {
        <form class="form-row invite-form" on:submit=create>
            <Show when=staff>
                <label class="field"><span>"Anzahl"</span><input type="number" min="1" max="50" bind:value=count/></label>
                <label class="field"><span>"Nutzungen je Code"</span><input type="number" min="1" max="1000" bind:value=uses/></label>
                <label class="field"><span>"Gültig (Tage, leer = unbegrenzt)"</span><input type="number" min="1" max="365" bind:value=days/></label>
            </Show>
            <label class="field grow"><span>"Notiz (optional)"</span><input type="text" maxlength="200" placeholder="z. B. für Anna" bind:value=note/></label>
            <button class="btn btn-primary" type="submit" disabled=move || busy.get()><Icon name="user-plus" size=16/>"Einladung erstellen"</button>
        </form>
        <label class="check small"><input type="checkbox" bind:checked=show_used/><span>"Verbrauchte und widerrufene anzeigen"</span></label>
        <ul class="plain-list invite-list">
            {move || {
                let now = chrono::Utc::now();
                let all = show_used.get();
                let list: Vec<InviteView> = local(invites).unwrap_or_default().into_iter().filter(|i| all || i.usable(now)).collect();
                if list.is_empty() {
                    return view! { <li class="muted small">"Keine Einladungen."</li> }.into_any();
                }
                list.into_iter().map(|i| {
                    let usable = i.usable(now);
                    let link = i.link(&origin());
                    let code = i.code.clone();
                    let status = if i.revoked { "widerrufen".to_string() }
                        else if i.uses >= i.max_uses { "verbraucht".to_string() }
                        else if i.expires_at.is_some_and(|e| e <= now) { "abgelaufen".to_string() }
                        else { format!("{} von {} genutzt", i.uses, i.max_uses) };
                    view! {
                        <li class="invite-row" class:inactive=!usable>
                            <code class="invite-code">{i.code.clone()}</code>
                            <div class="grow">
                                <div class="small">
                                    {status}
                                    {i.expires_at.map(|e| format!(" · gültig bis {}", local_datetime(e)))}
                                    {(!i.note.is_empty()).then(|| format!(" · {}", i.note))}
                                </div>
                                <div class="muted small row gap-sm wrap">
                                    {i.created_by.clone().map(|u| view! { "von " <Avatar user=u.clone() size=16/>{u.display_name.clone()} })}
                                    {(!i.used_by.is_empty()).then(|| view! {
                                        " · genutzt von "
                                        {i.used_by.iter().map(|u| view! { <a class="user-link" href=u.profile_url()>{u.display_name.clone()}</a>" " }).collect_view()}
                                    })}
                                </div>
                            </div>
                            {usable.then(|| view! {
                                <button class="btn btn-sm" on:click=move |_| { copy_to_clipboard(&link); toasts.success("Link kopiert."); }><Icon name="copy" size=14/>"Link"</button>
                                <button class="btn btn-sm btn-ghost" on:click=move |_| revoke(code.clone())>"Widerrufen"</button>
                            })}
                        </li>
                    }
                }).collect_view().into_any()
            }}
        </ul>
    }
}
