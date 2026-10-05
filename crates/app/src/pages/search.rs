use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_query_map};
use shared::search::SearchKind;

use crate::api::admin::search;
use crate::components::layout::urlencode;
use crate::components::{Avatar, EmptyState, ErrorBox, Icon, Spinner, TimeAgo};

#[component]
pub fn SearchPage() -> impl IntoView {
    let query = use_query_map();
    let navigate = use_navigate();
    let q = move || query.read().get("q").unwrap_or_default();
    let kind = move || query.read().get("kind").and_then(|k| SearchKind::parse(&k));
    let input = RwSignal::new(String::new());
    Effect::new(move |_| input.set(q()));

    let results = Resource::new(move || (q(), kind()), |(q, k)| search(q, k));

    let submit = {
        let navigate = navigate.clone();
        move |ev: leptos::ev::SubmitEvent| {
            ev.prevent_default();
            navigate(&format!("/search?q={}", urlencode(input.get_untracked().trim())), Default::default());
        }
    };

    view! {
        <Title text="Suche"/>
        <h1>"Suche"</h1>
        <form class="search-big" on:submit=submit>
            <Icon name="search"/>
            <input type="search" placeholder="Wonach suchst du?" bind:value=input autofocus=true/>
            <button class="btn btn-primary">"Suchen"</button>
        </form>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                let current_q = q();
                if current_q.trim().chars().count() < 2 {
                    return view! { <EmptyState icon="search" title="Gib mindestens zwei Zeichen ein."/> }.into_any();
                }
                match results.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(r) if r.counts.is_empty() => view! {
                        <EmptyState icon="search" title=format!("Nichts gefunden für „{current_q}“.")/>
                    }.into_any(),
                    Ok(r) => {
                        let total: i64 = r.counts.iter().map(|(_, n)| n).sum();
                        let active = kind();
                        let encoded = urlencode(&current_q);
                        view! {
                            <div class="tabs">
                                <a class="tab" class:active=active.is_none() href=format!("/search?q={encoded}")>
                                    "Alles " <span class="tab-count">{total}</span>
                                </a>
                                {r.counts.iter().map(|(k, n)| view! {
                                    <a class="tab" class:active=active == Some(*k) href=format!("/search?q={encoded}&kind={}", k.as_str())>
                                        {k.label()} " " <span class="tab-count">{*n}</span>
                                    </a>
                                }).collect_view()}
                            </div>
                            <ul class="search-results">
                                {r.hits.into_iter().map(|h| view! {
                                    <li class="search-hit card">
                                        <div class="hit-kind"><Icon name=h.kind.icon() size=15/>{h.kind.label()}</div>
                                        {h.thumb_url.clone().map(|t| view! { <img class="hit-thumb" src=t alt=""/> })}
                                        <a class="hit-title" href=h.link.clone()>{h.title.clone()}</a>
                                        <p class="hit-snippet" inner_html=h.snippet_html.clone()></p>
                                        <div class="hit-meta small muted">
                                            {h.author.clone().map(|a| view! { <Avatar user=a.clone() size=18/> <span>{a.display_name.clone()}</span> })}
                                            {h.created_at.map(|t| view! { <TimeAgo ts=t/> })}
                                        </div>
                                    </li>
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }
                }
            })}
        </Transition>
    }
}
