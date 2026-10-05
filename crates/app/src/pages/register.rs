use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::app::CurrentUser;
use crate::server_fns::auth_fns::Register;

#[component]
pub fn RegisterPage() -> impl IntoView {
    let register_action = ServerAction::<Register>::new();
    let CurrentUser(user_resource) = expect_context::<CurrentUser>();
    let navigate = use_navigate();

    Effect::new(move |_| {
        if let Some(Ok(_)) = register_action.value().get() {
            user_resource.refetch();
            navigate("/", Default::default());
        }
    });

    view! {
        <h1>"Registrieren"</h1>
        <ActionForm action=register_action>
            <label>
                "Benutzername"
                <input type="text" name="username" required minlength="3"/>
            </label>
            <label>
                "Anzeigename (optional)"
                <input type="text" name="display_name"/>
            </label>
            <label>
                "E-Mail"
                <input type="email" name="email" required/>
            </label>
            <label>
                "Passwort"
                <input type="password" name="password" required minlength="8"/>
            </label>
            <button type="submit">"Registrieren"</button>
        </ActionForm>
        {move || register_action.value().get().and_then(|r| r.err()).map(|e| view! {
            <p class="error">{e.to_string()}</p>
        })}
    }
}
