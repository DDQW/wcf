use leptos::html;
use leptos::prelude::*;

use crate::components::common::Spinner;

/// Invisible marker at the end of a list: calls `on_visible` when it
/// scrolls into view (IntersectionObserver), and again after each load if
/// it is still visible (short pages).
#[component]
pub fn InfiniteSentinel(
    #[prop(into)] on_visible: Callback<()>,
    /// Whether more items exist.
    #[prop(into)]
    enabled: Signal<bool>,
    #[prop(into)] loading: Signal<bool>,
    #[prop(optional, into)] done_text: String,
) -> impl IntoView {
    let node = NodeRef::<html::Div>::new();

    #[cfg(feature = "hydrate")]
    {
        use std::cell::RefCell;
        use std::rc::Rc;
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let visible = Rc::new(RefCell::new(false));
        type Observer = (web_sys::IntersectionObserver, Closure<dyn FnMut(js_sys::Array)>);
        let observer: StoredValue<Option<Observer>, LocalStorage> = StoredValue::new_local(None);

        let fire = move || {
            if enabled.get_untracked() && !loading.get_untracked() {
                on_visible.run(());
            }
        };

        Effect::new({
            let visible = visible.clone();
            move |_| {
                let Some(el) = node.get() else { return };
                if observer.with_value(|o| o.is_some()) {
                    return;
                }
                let visible = visible.clone();
                let cb = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
                    for entry in entries.iter() {
                        let entry: web_sys::IntersectionObserverEntry = entry.unchecked_into();
                        *visible.borrow_mut() = entry.is_intersecting();
                        if entry.is_intersecting() {
                            fire();
                        }
                    }
                });
                let opts = web_sys::IntersectionObserverInit::new();
                opts.set_root_margin("400px 0px");
                if let Ok(obs) = web_sys::IntersectionObserver::new_with_options(cb.as_ref().unchecked_ref(), &opts) {
                    obs.observe(&el);
                    observer.set_value(Some((obs, cb)));
                }
            }
        });

        // After a load finishes, keep going while the sentinel is visible.
        Effect::new({
            let visible = visible.clone();
            move |prev: Option<bool>| {
                let now_loading = loading.get();
                let has_more = enabled.get();
                if prev == Some(true) && !now_loading && has_more && *visible.borrow() {
                    let visible = visible.clone();
                    set_timeout(
                        move || {
                            if *visible.borrow() {
                                fire();
                            }
                        },
                        std::time::Duration::from_millis(50),
                    );
                }
                now_loading
            }
        });

        on_cleanup(move || {
            observer.update_value(|o| {
                if let Some((obs, _)) = o.take() {
                    obs.disconnect();
                }
            });
        });
    }

    #[cfg(not(feature = "hydrate"))]
    let _ = on_visible;

    view! {
        <div class="sentinel" node_ref=node>
            {move || {
                if loading.get() {
                    view! { <Spinner/> }.into_any()
                } else if !enabled.get() && !done_text.is_empty() {
                    view! { <p class="muted small">{done_text.clone()}</p> }.into_any()
                } else {
                    ().into_any()
                }
            }}
        </div>
    }
}
