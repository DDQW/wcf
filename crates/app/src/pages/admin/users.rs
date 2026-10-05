use leptos::prelude::*;
use shared::{AdminUserRow, Role};

use crate::api::admin::admin_users;
use crate::api::moderation::{
    admin_reset_link, set_user_active, set_user_quota, set_user_role, unban_user,
};
use crate::components::common::{copy_to_clipboard, sleep_ms};
use crate::components::{ErrorBox, Icon, Modal, Pager, RoleBadge, Spinner, TimeAgo};
use crate::state::{use_session, use_toasts};

#[component]
pub fn AdminUsers() -> impl IntoView {
    let search = RwSignal::new(String::new());
    let debounced = RwSignal::new(String::new());
    let filter = RwSignal::new("all".to_string());
    let page = RwSignal::new(0i64);
    let version = RwSignal::new(0u32);
    Effect::new(move |_| {
        let s = search.get();
        leptos::task::spawn_local(async move {
            sleep_ms(250).await;
            if search.get_untracked() == s {
                debounced.set(s);
                page.set(0);
            }
        });
    });
    let data = Resource::new(
        move || (debounced.get(), filter.get(), page.get(), version.get()),
        |(q, f, p, _)| admin_users((!q.is_empty()).then_some(q), f, p),
    );
    let total = Signal::derive(move || data.get().and_then(|r| r.ok()).map(|p| p.total).unwrap_or(0));
    let refresh = Callback::new(move |_| version.update(|v| *v += 1));

    view! {
        <h1>"Benutzer"</h1>
        <div class="toolbar card">
            <input type="search" placeholder="Name oder E-Mail …" bind:value=search/>
            <select bind:value=filter on:change=move |_| page.set(0)>
                <option value="all">"Alle"</option>
                <option value="staff">"Team"</option>
                <option value="banned">"Gesperrt"</option>
                <option value="inactive">"Deaktiviert"</option>
            </select>
        </div>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(p) => view! {
                        <div class="table-wrap card flush">
                            <table class="table">
                                <thead><tr>
                                    <th>"Benutzer"</th><th>"E-Mail"</th><th>"Rolle"</th><th>"Status"</th>
                                    <th>"Registriert"</th><th>"Zuletzt"</th><th>"Speicher"</th><th></th>
                                </tr></thead>
                                <tbody>
                                    {p.items.into_iter().map(|u| view! { <UserRow u=u on_change=refresh/> }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                    }.into_any(),
                }
            })}
        </Transition>
        <Pager page=page total=total per_page=30/>
    }
}

#[component]
fn UserRow(u: AdminUserRow, on_change: Callback<()>) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let id = u.id;
    let banned = u.banned_until.is_some_and(|t| t > chrono::Utc::now());
    let active = u.is_active;
    let reset_link = RwSignal::new(None::<String>);
    let link_open = RwSignal::new(false);
    let quota_open = RwSignal::new(false);
    let quota = RwSignal::new(String::new());

    let run = move |fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::error::AppError>>>>, ok: &'static str| {
        leptos::task::spawn_local(async move {
            match fut.await {
                Ok(()) => {
                    toasts.success(ok);
                    on_change.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let on_role = move |ev: leptos::ev::Event| {
        if let Some(r) = Role::parse(&event_target_value(&ev)) {
            run(Box::pin(set_user_role(id, r)), "Rolle geändert.");
        }
    };
    let gen_link = move |_| {
        leptos::task::spawn_local(async move {
            match admin_reset_link(id).await {
                Ok(link) => {
                    reset_link.set(Some(link));
                    link_open.set(true);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let save_quota = move |_| {
        let q = quota.get_untracked();
        let value = if q.trim().is_empty() { None } else { q.trim().parse::<i64>().ok() };
        quota_open.set(false);
        run(Box::pin(set_user_quota(id, value)), "Kontingent gespeichert.");
    };

    view! {
        <tr>
            <td>
                <a href=format!("/u/{}", u.username)><strong>{u.display_name.clone()}</strong></a>
                <div class="muted small">"@"{u.username.clone()}</div>
            </td>
            <td class="small">{u.email.clone()}</td>
            <td>
                {move || if session.is_admin() {
                    view! {
                        <select class="select-sm" on:change=on_role>
                            {Role::ALL.into_iter().map(|r| view! {
                                <option value=r.as_str() selected=r == u.role>{r.label()}</option>
                            }).collect_view()}
                        </select>
                    }.into_any()
                } else {
                    view! { <RoleBadge role=u.role/> <span class="small">{u.role.label()}</span> }.into_any()
                }}
            </td>
            <td class="small">
                {if !active {
                    view! { <span class="badge badge-muted">"deaktiviert"</span> }.into_any()
                } else if banned {
                    view! { <span class="badge badge-danger" title=u.ban_reason.clone().unwrap_or_default()>"gesperrt"</span> }.into_any()
                } else {
                    view! { <span class="badge badge-ok">"aktiv"</span> }.into_any()
                }}
            </td>
            <td class="small"><TimeAgo ts=u.created_at absolute=true/></td>
            <td class="small">{u.last_seen_at.map(|t| view! { <TimeAgo ts=t/> })}</td>
            <td class="small">{shared::util::format_bytes(u.storage_used)}</td>
            <td class="row gap-sm nowrap">
                {banned.then(|| view! {
                    <button class="btn btn-sm" title="Entsperren" on:click=move |_| run(Box::pin(unban_user(id)), "Sperre aufgehoben.")>
                        <Icon name="unlock" size=14/>
                    </button>
                })}
                <Show when=move || session.is_admin()>
                    <button class="btn btn-sm" title=if active { "Deaktivieren" } else { "Aktivieren" }
                        on:click=move |_| run(Box::pin(set_user_active(id, !active)), if active { "Konto deaktiviert." } else { "Konto aktiviert." })>
                        <Icon name=if active { "user-x" } else { "check" } size=14/>
                    </button>
                    <button class="btn btn-sm" title="Passwort-Reset-Link" on:click=gen_link><Icon name="link" size=14/></button>
                    <button class="btn btn-sm" title="Speicherkontingent" on:click=move |_| quota_open.set(true)><Icon name="hard-drive" size=14/></button>
                </Show>
                <Modal open=link_open title="Passwort-Reset-Link">
                    <div class="stack">
                        <p class="small">"Der Link ist 2 Stunden gültig und kann einmal benutzt werden. Gib ihn dem Benutzer auf sicherem Weg weiter."</p>
                        <input type="text" readonly prop:value=move || reset_link.get().unwrap_or_default()/>
                        <div class="row end">
                            <button class="btn btn-primary" on:click=move |_| {
                                if let Some(l) = reset_link.get_untracked() { copy_to_clipboard(&l); toasts.success("Kopiert."); }
                            }><Icon name="copy" size=16/>"Kopieren"</button>
                        </div>
                    </div>
                </Modal>
                <Modal open=quota_open title="Speicherkontingent">
                    <div class="stack">
                        <label class="field">
                            <span>"Kontingent in MB (leer = Standard der Seite, 0 = unbegrenzt)"</span>
                            <input type="number" min="0" bind:value=quota/>
                        </label>
                        <div class="row end"><button class="btn btn-primary" on:click=save_quota>"Speichern"</button></div>
                    </div>
                </Modal>
            </td>
        </tr>
    }
}
