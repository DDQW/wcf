use leptos::prelude::*;
use leptos_meta::Title;
use shared::NotificationView;

use crate::api::notifications::{clear_notifications, list_notifications, mark_all_notifications_read};
use crate::components::layout::NotificationRow;
use crate::components::{EmptyState, Icon, InfiniteSentinel, PageHeader, RequireLogin};
use crate::components::common::confirm;
use crate::state::{use_session, use_toasts};

#[component]
pub fn NotificationsPage() -> impl IntoView {
    view! {
        <Title text="Benachrichtigungen"/>
        <RequireLogin>
            <NotificationList/>
        </RequireLogin>
    }
}

#[component]
fn NotificationList() -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let items = RwSignal::new(Vec::<NotificationView>::new());
    let cursor = RwSignal::new(None::<String>);
    let has_more = RwSignal::new(true);
    let loading = RwSignal::new(false);
    let unread_only = RwSignal::new(false);

    let load = move |reset: bool| {
        if loading.get_untracked() {
            return;
        }
        loading.set(true);
        let c = if reset { None } else { cursor.get_untracked() };
        leptos::task::spawn_local(async move {
            match list_notifications(c, unread_only.get_untracked()).await {
                Ok(page) => {
                    if reset {
                        items.set(page.items);
                    } else {
                        items.update(|v| v.extend(page.items));
                    }
                    has_more.set(page.next_cursor.is_some());
                    cursor.set(page.next_cursor);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            loading.set(false);
        });
    };
    Effect::new(move |_| {
        unread_only.track();
        load(true);
    });

    let mark_all = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(u) = mark_all_notifications_read().await {
                session.unread.set(u);
                items.update(|v| v.iter_mut().for_each(|n| n.is_read = true));
            }
        });
    };
    let clear = move |_| {
        if !confirm("Alle Benachrichtigungen löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            if clear_notifications().await.is_ok() {
                items.set(Vec::new());
                session.unread.set(0);
            }
        });
    };

    view! {
        <PageHeader title="Benachrichtigungen">
            <label class="check"><input type="checkbox" bind:checked=unread_only/>"Nur ungelesene"</label>
            <button class="btn" on:click=mark_all><Icon name="check" size=16/>"Alle gelesen"</button>
            <button class="btn btn-ghost" on:click=clear><Icon name="trash" size=16/>"Leeren"</button>
        </PageHeader>
        <div class="card flush">
            <Show when=move || !(items.with(|i| i.is_empty()) && !loading.get()) fallback=|| view! {
                <EmptyState icon="inbox" title="Keine Benachrichtigungen."/>
            }>
                <ul class="notif-list big">
                    <For each=move || items.get() key=|n| (n.id, n.is_read) children=|n| view! { <NotificationRow n=n/> }/>
                </ul>
            </Show>
            <InfiniteSentinel on_visible=move |_| load(false) enabled=has_more loading=loading/>
        </div>
    }
}
