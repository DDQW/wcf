//! Start-page sections contributed by optional modules.

use leptos::prelude::*;
use shared::HomeExtra;

use crate::components::{Avatar, Icon, TimeAgo};

#[allow(non_snake_case)]
pub fn HomeExtraView(extra: HomeExtra) -> impl IntoView {
    view! {
        {(!extra.latest_posts.is_empty()).then(|| view! {
            <section class="card">
                <div class="card-head">
                    <h2><Icon name="book-open"/>"Aus den Blogs"</h2>
                    <a href="/blogs" class="small">"Alle"</a>
                </div>
                <ul class="plain-list home-posts">
                    {extra.latest_posts.into_iter().map(|p| view! {
                        <li>
                            <a class="home-post" href=p.url()>
                                {p.cover_url.clone().map(|c| view! { <img src=c alt="" loading="lazy"/> })}
                                <div>
                                    <strong>{p.title.clone()}</strong>
                                    <div class="muted small row gap-sm">
                                        <Avatar user=p.author.clone() size=16/>{p.author.display_name.clone()}
                                        {p.published_at.map(|t| view! { " · " <TimeAgo ts=t/> })}
                                    </div>
                                </div>
                            </a>
                        </li>
                    }).collect_view()}
                </ul>
            </section>
        })}
    }
}
