//! Admin navigation entries contributed by optional modules.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::components::Icon;
use crate::state::use_session;

pub fn admin_nav() -> impl IntoView {
    let session = use_session();
    let location = use_location();
    let link = move |href: &'static str, icon: &'static str, label: &'static str| {
        let active = move || location.pathname.get().starts_with(href);
        view! { <a class="subnav-item" class:active=active href=href><Icon name=icon size=16/>{label}</a> }
    };
    view! {
        {link("/admin/invites", "user-plus", "Einladungen")}
        <Show when=move || session.is_admin() && session.site.with(|s| s.modules.cms)>
            {link("/admin/cms", "layout", "Inhaltstypen")}
            {link("/admin/menu", "menu", "Menü")}
        </Show>
    }
}
