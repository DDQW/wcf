use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;
use shared::user::ProfileView;
use shared::{Role, SubjectType};

use crate::api::moderation::{ban_user, set_user_role, unban_user};
use crate::api::users::{profile, user_activity};
use crate::components::{
    Avatar, EmptyState, ErrorBox, Icon, Modal, ReportButton, RoleBadge, Spinner, TimeAgo,
};
use crate::pages::not_found::Missing;
use crate::realtime::use_realtime;
use crate::state::{use_session, use_toasts};

#[component]
pub fn ProfilePage() -> impl IntoView {
    let params = use_params_map();
    let username = move || params.read().get("username").unwrap_or_default();
    let data = Resource::new(username, profile);
    let reload = move || data.refetch();

    view! {
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Ok(p) => view! { <ProfileContent p=p on_change=Callback::new(move |_| reload())/> }.into_any(),
                    Err(e) if e.is_not_found() => view! { <Missing text="Diesen Benutzer gibt es nicht."/> }.into_any(),
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn ProfileContent(p: ProfileView, on_change: Callback<()>) -> impl IntoView {
    let session = use_session();
    let rt = use_realtime();
    let user = p.user.clone();
    let user_id = user.id;
    let is_self = move || session.user_id() == Some(user_id);
    let activity = Resource::new(move || user_id, user_activity);
    let stats = p.stats.clone();
    let banned = p.banned_until.is_some();
    let target_role = user.role;

    view! {
        <Title text=user.display_name.clone()/>
        <section class="card profile-head">
            <Avatar user=user.clone() size=96 presence=true/>
            <div class="profile-info">
                <h1>{user.display_name.clone()} <RoleBadge role=user.role/></h1>
                <p class="muted">"@"{user.username.clone()}</p>
                <div class="profile-meta">
                    <span><Icon name="calendar" size=15/>"Dabei seit " <TimeAgo ts=p.created_at absolute=true/></span>
                    {move || if rt.is_online(user_id) {
                        view! { <span class="online-text"><Icon name="radio" size=15/>"Online"</span> }.into_any()
                    } else {
                        p.last_seen_at.map(|t| view! {
                            <span><Icon name="clock" size=15/>"Zuletzt online " <TimeAgo ts=t/></span>
                        }.into_any()).unwrap_or_else(|| ().into_any())
                    }}
                    {p.location.clone().map(|l| view! { <span><Icon name="map-pin" size=15/>{l}</span> })}
                    {p.website.clone().map(|w| view! {
                        <a href=w.clone() target="_blank" rel="noopener noreferrer nofollow"><Icon name="globe" size=15/>{w.trim_start_matches("https://").trim_start_matches("http://").to_string()}</a>
                    })}
                </div>
                {banned.then(|| view! {
                    <div class="alert alert-error small">
                        <Icon name="lock" size=16/>
                        "Gesperrt"
                        {p.banned_until.map(|t| view! { " bis " <TimeAgo ts=t absolute=true/> })}
                        {p.ban_reason.clone().map(|r| format!(" – {r}"))}
                    </div>
                })}
            </div>
            <div class="profile-actions">
                {p.has_blog.then(|| view! {
                    <a class="btn" href=format!("/blog/{}", user.username)><Icon name="book-open" size=16/>"Blog"</a>
                })}
                <Show when=is_self>
                    <a class="btn" href="/settings"><Icon name="edit" size=16/>"Profil bearbeiten"</a>
                </Show>
                <Show when=move || !is_self() && session.logged_in()>
                    {crate::pages::profile_ext::profile_actions(user_id)}
                    <ReportButton subject_type=SubjectType::User subject_id=user_id/>
                </Show>
                <Show when=move || !is_self() && session.is_mod()>
                    <ModTools user_id=user_id banned=banned role=target_role on_change=on_change/>
                </Show>
            </div>
        </section>

        <div class="stat-row">
            <div class="stat"><span class="stat-value">{stats.topics}</span><span class="stat-label">"Themen"</span></div>
            <div class="stat"><span class="stat-value">{stats.posts}</span><span class="stat-label">"Beiträge"</span></div>
            <div class="stat"><span class="stat-value">{stats.blog_posts}</span><span class="stat-label">"Blogposts"</span></div>
            <a class="stat" href=format!("/gallery?tab=latest&user={user_id}")><span class="stat-value">{stats.images}</span><span class="stat-label">"Bilder"</span></a>
            <div class="stat"><span class="stat-value">{stats.reactions_received}</span><span class="stat-label">"Reaktionen"</span></div>
        </div>

        <div class="profile-grid">
            <section class="card">
                <h2>"Über mich"</h2>
                {if p.bio_html.trim().is_empty() {
                    view! { <p class="muted">"Noch keine Beschreibung."</p> }.into_any()
                } else {
                    view! { <div class="prose" inner_html=p.bio_html.clone()></div> }.into_any()
                }}
            </section>
            <section class="card">
                <h2>"Aktivität"</h2>
                <Suspense fallback=|| view! { <Spinner/> }>
                    {move || Suspend::new(async move {
                        match activity.await {
                            Ok(items) if items.is_empty() => view! {
                                <EmptyState icon="activity" title="Noch keine Aktivität."/>
                            }.into_any(),
                            Ok(items) => view! {
                                <ul class="activity-list">
                                    {items.into_iter().map(|a| view! {
                                        <li>
                                            <Icon name=a.icon.clone() size=16/>
                                            <div>
                                                <div class="small muted">{a.label.clone()} " · " <TimeAgo ts=a.created_at/></div>
                                                <a href=a.link.clone() class="activity-title">{a.title.clone()}</a>
                                                <p class="activity-excerpt">{a.excerpt.clone()}</p>
                                            </div>
                                        </li>
                                    }).collect_view()}
                                </ul>
                            }.into_any(),
                            Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        }
                    })}
                </Suspense>
            </section>
        </div>
    }
}

#[component]
fn ModTools(user_id: uuid::Uuid, banned: bool, role: Role, on_change: Callback<()>) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let ban_open = RwSignal::new(false);
    let days = RwSignal::new("7".to_string());
    let reason = RwSignal::new(String::new());
    let new_role = RwSignal::new(role.as_str().to_string());

    let do_ban = move |_| {
        let d: Option<i64> = days.get_untracked().parse().ok().filter(|d: &i64| *d > 0);
        leptos::task::spawn_local(async move {
            match ban_user(user_id, d, reason.get_untracked()).await {
                Ok(()) => {
                    toasts.success("Benutzer gesperrt.");
                    ban_open.set(false);
                    on_change.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let do_unban = move |_| {
        leptos::task::spawn_local(async move {
            match unban_user(user_id).await {
                Ok(()) => {
                    toasts.success("Sperre aufgehoben.");
                    on_change.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let change_role = move |ev: leptos::ev::Event| {
        let value = event_target_value(&ev);
        new_role.set(value.clone());
        let Some(r) = Role::parse(&value) else { return };
        leptos::task::spawn_local(async move {
            match set_user_role(user_id, r).await {
                Ok(()) => {
                    toasts.success("Rolle geändert.");
                    on_change.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };

    view! {
        {if banned {
            view! { <button class="btn" on:click=do_unban><Icon name="unlock" size=16/>"Entsperren"</button> }.into_any()
        } else {
            view! { <button class="btn btn-danger-soft" on:click=move |_| ban_open.set(true)><Icon name="user-x" size=16/>"Sperren"</button> }.into_any()
        }}
        <Show when=move || session.is_admin()>
            <select class="select-sm" title="Rolle" prop:value=move || new_role.get() on:change=change_role>
                {Role::ALL.into_iter().map(|r| view! { <option value=r.as_str()>{r.label()}</option> }).collect_view()}
            </select>
        </Show>
        <Modal open=ban_open title="Benutzer sperren">
            <div class="stack">
                <label class="field">
                    <span>"Dauer"</span>
                    <select bind:value=days>
                        <option value="1">"1 Tag"</option>
                        <option value="3">"3 Tage"</option>
                        <option value="7">"7 Tage"</option>
                        <option value="30">"30 Tage"</option>
                        <option value="365">"1 Jahr"</option>
                        <option value="0">"Dauerhaft"</option>
                    </select>
                </label>
                <label class="field">
                    <span>"Grund (sieht der Benutzer beim Anmelden)"</span>
                    <textarea rows=3 bind:value=reason></textarea>
                </label>
                <div class="row gap end">
                    <button class="btn" on:click=move |_| ban_open.set(false)>"Abbrechen"</button>
                    <button class="btn btn-danger" on:click=do_ban>"Sperren"</button>
                </div>
            </div>
        </Modal>
    }
}
