use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::app::CurrentUser;
use crate::server_fns::auth_fns::Login;

#[component]
pub fn LoginPage() -> impl IntoView {
    let login_action = ServerAction::<Login>::new();
    let CurrentUser(user_resource) = expect_context::<CurrentUser>();
    let navigate = use_navigate();

    Effect::new(move |_| {
        if let Some(Ok(_)) = login_action.value().get() {
            user_resource.refetch();
            navigate("/", Default::default());
        }
    });

    view! {
        <h1>"Anmelden"</h1>
        <ActionForm action=login_action>
            <label>
                "Benutzername oder E-Mail"
                <input type="text" name="username_or_email" required/>
            </label>
            <label>
                "Passwort"
                <input type="password" name="password" required/>
            </label>
            <button type="submit">"Anmelden"</button>
        </ActionForm>
        {move || login_action.value().get().and_then(|r| r.err()).map(|e| view! {
            <p class="error">{e.to_string()}</p>
        })}
    }
}
