use leptos::prelude::*;
use shared::moderation::ReportReason;
use shared::{ReactionCount, SubjectType, REACTIONS};
use uuid::Uuid;

use crate::api::moderation::{reaction_users, report_content, toggle_reaction};
use crate::components::icon::Icon;
use crate::components::modal::Modal;
use crate::state::{use_session, use_toasts};

/// Emoji reactions with counts; click toggles your own reaction.
#[component]
pub fn ReactionBar(
    subject_type: SubjectType,
    subject_id: Uuid,
    reactions: Vec<ReactionCount>,
) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let counts = RwSignal::new(reactions);
    let picker = RwSignal::new(false);
    let busy = RwSignal::new(false);

    let toggle = move |emoji: String| {
        if !session.logged_in() {
            toasts.info("Melde dich an, um zu reagieren.", Some("/login".into()));
            return;
        }
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        picker.set(false);
        leptos::task::spawn_local(async move {
            match toggle_reaction(subject_type, subject_id, emoji).await {
                Ok(c) => counts.set(c),
                Err(e) => toasts.error(e.to_string()),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="reactions">
            <For
                each=move || counts.get()
                key=|c| (c.emoji.clone(), c.count, c.me)
                children=move |c| {
                    let emoji = c.emoji.clone();
                    let emoji_for_title = c.emoji.clone();
                    let title = RwSignal::new(String::new());
                    let load_title = move |_| {
                        if !title.get_untracked().is_empty() {
                            return;
                        }
                        let emoji = emoji_for_title.clone();
                        leptos::task::spawn_local(async move {
                            if let Ok(users) = reaction_users(subject_type, subject_id, emoji).await {
                                let names: Vec<String> = users.iter().take(8).map(|u| u.display_name.clone()).collect();
                                let more = users.len().saturating_sub(8);
                                let mut text = names.join(", ");
                                if more > 0 {
                                    text.push_str(&format!(" und {more} weitere"));
                                }
                                title.set(text);
                            }
                        });
                    };
                    view! {
                        <button
                            class="reaction"
                            class:mine=c.me
                            title=move || title.get()
                            on:mouseenter=load_title
                            on:click=move |_| toggle(emoji.clone())
                        >
                            <span class="emoji">{c.emoji.clone()}</span>
                            <span class="count">{c.count}</span>
                        </button>
                    }
                }
            />
            <div class="reaction-add-wrap">
                <button class="reaction reaction-add" title="Reagieren" aria-label="Reagieren"
                    on:click=move |_| picker.update(|p| *p = !*p)>
                    <Icon name="smile" size=16/>
                </button>
                <Show when=move || picker.get()>
                    <div class="reaction-picker">
                        {REACTIONS.iter().map(|e| {
                            let emoji = e.to_string();
                            view! {
                                <button class="reaction-option" on:click=move |_| toggle(emoji.clone())>{*e}</button>
                            }
                        }).collect_view()}
                    </div>
                </Show>
            </div>
        </div>
    }
}

/// "Melden" button with a reason dialog.
#[component]
pub fn ReportButton(
    subject_type: SubjectType,
    subject_id: Uuid,
    #[prop(optional)] compact: bool,
) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let open = RwSignal::new(false);
    let reason = RwSignal::new(ReportReason::Spam);
    let details = RwSignal::new(String::new());
    let sending = RwSignal::new(false);

    let submit = move |_| {
        sending.set(true);
        leptos::task::spawn_local(async move {
            match report_content(subject_type, subject_id, reason.get_untracked(), details.get_untracked()).await {
                Ok(()) => {
                    toasts.success("Danke! Die Moderation sieht sich das an.");
                    open.set(false);
                    details.set(String::new());
                }
                Err(e) => toasts.error(e.to_string()),
            }
            sending.set(false);
        });
    };

    view! {
        <Show when=move || session.logged_in()>
            <button class="action-btn" title="Melden" on:click=move |_| open.set(true)>
                <Icon name="flag" size=15/>
                {(!compact).then_some("Melden")}
            </button>
        </Show>
        <Modal open=open title=format!("{} melden", subject_type.label())>
            <div class="stack">
                <fieldset class="radio-list">
                    {ReportReason::ALL.into_iter().map(|r| view! {
                        <label class="radio">
                            <input type="radio" name="report-reason"
                                checked=move || reason.get() == r
                                on:change=move |_| reason.set(r)/>
                            {r.label()}
                        </label>
                    }).collect_view()}
                </fieldset>
                <label class="field">
                    <span>"Details (optional)"</span>
                    <textarea rows=3 bind:value=details placeholder="Was ist das Problem?"></textarea>
                </label>
                <div class="row gap end">
                    <button class="btn" on:click=move |_| open.set(false)>"Abbrechen"</button>
                    <button class="btn btn-danger" disabled=move || sending.get() on:click=submit>
                        <Icon name="flag" size=16/>"Melden"
                    </button>
                </div>
            </div>
        </Modal>
    }
}
