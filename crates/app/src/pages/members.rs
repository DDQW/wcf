use leptos::prelude::*;
use leptos_meta::Title;
use shared::user::MemberSort;
use shared::Role;

use crate::api::users::members;
use crate::components::common::sleep_ms;
use crate::components::{Avatar, EmptyState, ErrorBox, PageHeader, Pager, RoleBadge, Spinner, TimeAgo};

#[component]
pub fn MembersPage() -> impl IntoView {
    let search = RwSignal::new(String::new());
    let debounced = RwSignal::new(String::new());
    let role = RwSignal::new(String::new());
    let online = RwSignal::new(false);
    let sort = RwSignal::new("newest".to_string());
    let page = RwSignal::new(0i64);

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
        move || (debounced.get(), role.get(), online.get(), sort.get(), page.get()),
        |(q, role, online, sort, page)| async move {
            let sort = match sort.as_str() {
                "name" => MemberSort::Name,
                "active" => MemberSort::Active,
                "posts" => MemberSort::Posts,
                _ => MemberSort::Newest,
            };
            members((!q.is_empty()).then_some(q), Role::parse(&role), online, sort, page).await
        },
    );
    let total = Signal::derive(move || data.get().and_then(|r| r.ok()).map(|p| p.total).unwrap_or(0));

    view! {
        <Title text="Mitglieder"/>
        <PageHeader title="Mitglieder"/>
        <div class="toolbar card">
            <input type="search" placeholder="Name suchen …" bind:value=search/>
            <select bind:value=role on:change=move |_| page.set(0)>
                <option value="">"Alle Rollen"</option>
                <option value="admin">"Admins"</option>
                <option value="moderator">"Moderatoren"</option>
                <option value="member">"Mitglieder"</option>
            </select>
            <select bind:value=sort>
                <option value="newest">"Neueste zuerst"</option>
                <option value="name">"Name"</option>
                <option value="active">"Zuletzt aktiv"</option>
                <option value="posts">"Meiste Beiträge"</option>
            </select>
            <label class="check">
                <input type="checkbox" bind:checked=online on:change=move |_| page.set(0)/>
                "Nur online"
            </label>
        </div>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(p) if p.items.is_empty() => view! { <EmptyState icon="users" title="Keine Mitglieder gefunden."/> }.into_any(),
                    Ok(p) => view! {
                        <div class="member-grid">
                            {p.items.into_iter().map(|m| view! {
                                <a class="member-card card" href=m.user.profile_url()>
                                    <Avatar user=m.user.clone() size=56 presence=true/>
                                    <div class="member-name">{m.user.display_name.clone()} <RoleBadge role=m.user.role/></div>
                                    <div class="muted small">"@"{m.user.username.clone()}</div>
                                    <div class="muted small">{m.post_count} " Beiträge · seit " <TimeAgo ts=m.created_at absolute=false/></div>
                                </a>
                            }).collect_view()}
                        </div>
                    }.into_any(),
                }
            })}
        </Transition>
        <Pager page=page total=total per_page=24/>
    }
}
