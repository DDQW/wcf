use leptos::prelude::*;

use crate::components::icon::Icon;

/// Centered dialog. Closes on backdrop click and Escape.
#[component]
pub fn Modal(
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    children: ChildrenFn,
    #[prop(optional)] wide: bool,
) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    {
        let handle = window_event_listener(leptos::ev::keydown, move |ev| {
            if ev.key() == "Escape" && open.get_untracked() {
                open.set(false);
            }
        });
        on_cleanup(move || handle.remove());
    }
    let title = StoredValue::new(title);
    let children = StoredValue::new(children);
    view! {
        <Show when=move || open.get()>
            <div class="modal-backdrop" on:click=move |_| open.set(false)>
                <div
                    class="modal"
                    class:modal-wide=wide
                    role="dialog"
                    aria-modal="true"
                    on:click=|ev| ev.stop_propagation()
                >
                    <header class="modal-header">
                        <h2>{title.get_value()}</h2>
                        <button class="icon-btn" aria-label="Schließen" on:click=move |_| open.set(false)>
                            <Icon name="x"/>
                        </button>
                    </header>
                    <div class="modal-body">{children.with_value(|c| c())}</div>
                </div>
            </div>
        </Show>
    }
}

/// Click-to-open dropdown menu anchored to its trigger.
#[component]
pub fn Dropdown(
    /// Content of the trigger button.
    trigger: ChildrenFn,
    children: ChildrenFn,
    #[prop(optional, into)] class: String,
    #[prop(optional)] align_right: bool,
    #[prop(optional, into)] label: String,
) -> impl IntoView {
    let open = RwSignal::new(false);
    #[cfg(feature = "hydrate")]
    {
        let handle = window_event_listener(leptos::ev::click, move |_| {
            if open.get_untracked() {
                open.set(false);
            }
        });
        on_cleanup(move || handle.remove());
    }
    let trigger = StoredValue::new(trigger);
    let children = StoredValue::new(children);
    view! {
        <div class=format!("dropdown {class}") on:click=|ev| ev.stop_propagation()>
            <button
                class="dropdown-trigger"
                aria-label=label
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {trigger.with_value(|t| t())}
            </button>
            <Show when=move || open.get()>
                <div class="dropdown-menu" class:align-right=align_right on:click=move |_| open.set(false)>
                    {children.with_value(|c| c())}
                </div>
            </Show>
        </div>
    }
}
