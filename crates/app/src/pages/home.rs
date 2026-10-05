use leptos::prelude::*;
use leptos_router::components::A;

use crate::app::CurrentUser;
use crate::server_fns::forum_fns::{list_categories, CreateCategory};

#[component]
pub fn HomePage() -> impl IntoView {
    let categories = Resource::new(|| (), |_| list_categories());
    let CurrentUser(user_resource) = expect_context::<CurrentUser>();

    let create_category_action = ServerAction::<CreateCategory>::new();
    Effect::new(move |_| {
        if create_category_action.value().get().is_some() {
            categories.refetch();
        }
    });

    view! {
        <h1>"Forum"</h1>

        <Suspense fallback=move || view! { <p>"Lade Kategorien..."</p> }>
            {move || Suspend::new(async move {
                match categories.await {
                    Ok(cats) if cats.is_empty() => view! { <p>"Noch keine Kategorien."</p> }.into_any(),
                    Ok(cats) => view! {
                        <ul class="category-list">
                            <For each=move || cats.clone() key=|c| c.id let(cat)>
                                <li>
                                    <A href=format!("/forum/category/{}", cat.id)>{cat.name.clone()}</A>
                                    {cat.description.clone().map(|d| view! { <p class="muted">{d}</p> })}
                                </li>
                            </For>
                        </ul>
                    }.into_any(),
                    Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                }
            })}
        </Suspense>

        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                let is_admin = matches!(user_resource.await, Ok(Some(u)) if u.is_admin());
                is_admin.then(|| view! {
                    <details class="new-category">
                        <summary>"Neue Kategorie anlegen"</summary>
                        <ActionForm action=create_category_action>
                            <input type="text" name="name" placeholder="Name" required/>
                            <input type="text" name="description" placeholder="Beschreibung (optional)"/>
                            <button type="submit">"Anlegen"</button>
                        </ActionForm>
                    </details>
                })
            })}
        </Suspense>
    }
}
