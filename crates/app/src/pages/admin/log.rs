use leptos::prelude::*;
use shared::moderation::mod_action_label;

use crate::api::moderation::mod_log;
use crate::components::{ErrorBox, Pager, Spinner, TimeAgo};

#[component]
pub fn AdminLog() -> impl IntoView {
    let page = RwSignal::new(0i64);
    let data = Resource::new(move || page.get(), mod_log);
    let total = Signal::derive(move || data.get().and_then(|r| r.ok()).map(|p| p.total).unwrap_or(0));
    view! {
        <h1>"Moderationsprotokoll"</h1>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(p) => view! {
                        <div class="table-wrap card flush">
                            <table class="table">
                                <thead><tr><th>"Zeit"</th><th>"Wer"</th><th>"Aktion"</th><th>"Betroffen"</th><th>"Details"</th></tr></thead>
                                <tbody>
                                    {p.items.into_iter().map(|e| view! {
                                        <tr>
                                            <td class="small nowrap"><TimeAgo ts=e.created_at absolute=true/></td>
                                            <td>{e.actor.clone().map(|a| view! { <a href=a.profile_url()>{a.display_name.clone()}</a> })}</td>
                                            <td>{mod_action_label(&e.action).to_string()}</td>
                                            <td>{e.target_user.clone().map(|a| view! { <a href=a.profile_url()>{a.display_name.clone()}</a> })}</td>
                                            <td class="small">{e.details.clone()}</td>
                                        </tr>
                                    }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                    }.into_any(),
                }
            })}
        </Transition>
        <Pager page=page total=total per_page=50/>
    }
}
