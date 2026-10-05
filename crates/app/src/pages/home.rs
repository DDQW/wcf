use leptos::prelude::*;
use leptos_meta::Title;

use crate::api::users::fetch_home_data;
use crate::components::{Avatar, ErrorBox, Icon, Spinner};
use crate::pages::forum::list::TopicRow;
use crate::state::use_session;

#[component]
pub fn HomePage() -> impl IntoView {
    let session = use_session();
    let data = Resource::new(|| (), |_| fetch_home_data());
    view! {
        <Title text=""/>
        <section class="hero card">
            <div class="hero-text">
                <h1>{move || session.site.with(|s| s.site_name.clone())}</h1>
                <div class="prose" inner_html=move || session.site.with(|s| s.welcome_html.clone())></div>
                <Show when=move || !session.logged_in()>
                    <div class="row gap">
                        <a class="btn btn-primary" href="/register"><Icon name="user-plus" size=17/>"Mitmachen"</a>
                        <a class="btn" href="/login">"Anmelden"</a>
                    </div>
                </Show>
            </div>
        </section>
        <Suspense fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(d) => {
                        let stats = d.stats.clone();
                        let extra = d.extra.clone();
                        view! {
                            <div class="stat-row">
                                <div class="stat"><span class="stat-value">{stats.members}</span><span class="stat-label">"Mitglieder"</span></div>
                                <div class="stat"><span class="stat-value">{stats.topics}</span><span class="stat-label">"Themen"</span></div>
                                <div class="stat"><span class="stat-value">{stats.posts}</span><span class="stat-label">"Beiträge"</span></div>
                                <div class="stat"><span class="stat-value">{d.online.len()}</span><span class="stat-label">"online"</span></div>
                            </div>
                            <div class="home-grid">
                                <section class="card">
                                    <div class="card-head">
                                        <h2><Icon name="message-square"/>"Neueste Themen"</h2>
                                        <a href="/forum" class="small">"Zum Forum"</a>
                                    </div>
                                    {if d.latest_topics.is_empty() {
                                        view! { <p class="muted">"Noch keine Themen."</p> }.into_any()
                                    } else {
                                        view! {
                                            <div class="topic-list compact">
                                                {d.latest_topics.into_iter().map(|t| view! { <TopicRow topic=t compact=true/> }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                </section>
                                <aside class="stack">
                                    {crate::pages::home_extra::HomeExtraView(extra)}
                                    <section class="card">
                                        <h2><Icon name="radio"/>"Gerade online"</h2>
                                        {if d.online.is_empty() {
                                            view! { <p class="muted small">"Niemand – sei der Erste!"</p> }.into_any()
                                        } else {
                                            view! {
                                                <div class="avatar-cloud">
                                                    {d.online.into_iter().map(|u| view! {
                                                        <a href=u.profile_url() title=u.display_name.clone()>
                                                            <Avatar user=u.clone() size=34 presence=true/>
                                                        </a>
                                                    }).collect_view()}
                                                </div>
                                            }.into_any()
                                        }}
                                    </section>
                                    <section class="card">
                                        <h2><Icon name="user-plus"/>"Neue Mitglieder"</h2>
                                        <ul class="plain-list">
                                            {d.newest_members.into_iter().map(|u| view! {
                                                <li class="row gap-sm">
                                                    <Avatar user=u.clone() size=26/>
                                                    <a href=u.profile_url()>{u.display_name.clone()}</a>
                                                </li>
                                            }).collect_view()}
                                        </ul>
                                    </section>
                                </aside>
                            </div>
                        }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}
