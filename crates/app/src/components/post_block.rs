use leptos::prelude::*;
use shared::PostView;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A top-level post plus all of its (possibly nested) replies, in
/// chronological order, ready to render as one collapsible unit.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadBlock {
    pub thread_root_id: Uuid,
    pub posts: Vec<PostView>,
}

/// Groups a flat, chronologically-ordered post list into thread blocks,
/// preserving the order in which each thread's root first appeared.
pub fn group_into_blocks(posts: &[PostView]) -> Vec<ThreadBlock> {
    let mut order: Vec<Uuid> = Vec::new();
    let mut map: HashMap<Uuid, Vec<PostView>> = HashMap::new();
    for p in posts {
        map.entry(p.thread_root_id).or_insert_with(|| {
            order.push(p.thread_root_id);
            Vec::new()
        });
        map.get_mut(&p.thread_root_id).unwrap().push(p.clone());
    }
    order
        .into_iter()
        .map(|root_id| {
            let mut ps = map.remove(&root_id).unwrap_or_default();
            ps.sort_by_key(|p| p.created_at);
            ThreadBlock {
                thread_root_id: root_id,
                posts: ps,
            }
        })
        .collect()
}

/// Blocks with more replies than this are collapsed by default the first
/// time they are seen (a deliberate MVP simplification: this state is
/// client-only and resets on reload).
pub const COLLAPSE_THRESHOLD: usize = 3;

#[component]
pub fn ThreadBlockView(
    block: ThreadBlock,
    collapsed: RwSignal<HashSet<Uuid>>,
    on_reply: Callback<Uuid>,
) -> impl IntoView {
    let root_id = block.thread_root_id;
    let reply_count = block.posts.len().saturating_sub(1);
    let is_collapsed = move || collapsed.get().contains(&root_id);
    let toggle = move |_| {
        collapsed.update(|set| {
            if !set.remove(&root_id) {
                set.insert(root_id);
            }
        });
    };

    let mut iter = block.posts.into_iter();
    let root_post = iter.next();
    let replies: Vec<PostView> = iter.collect();

    view! {
        <div class="thread-block">
            {root_post.map(|p| view! { <PostRow post=p on_reply=on_reply/> })}
            {(reply_count > 0).then(|| {
                view! {
                    <button class="collapse-toggle" on:click=toggle>
                        {move || if is_collapsed() {
                            format!("{reply_count} Antworten anzeigen")
                        } else {
                            "Antworten einklappen".to_string()
                        }}
                    </button>
                }
            })}
            <div class="replies" class:hidden=is_collapsed>
                <For each=move || replies.clone() key=|p| p.id let(post)>
                    {
                        let margin = format!("{}px", (post.depth as i32).min(6) * 20);
                        view! {
                            <div class="reply" style:margin-left=margin>
                                <PostRow post=post on_reply=on_reply/>
                            </div>
                        }
                    }
                </For>
            </div>
        </div>
    }
}

#[component]
fn PostRow(post: PostView, on_reply: Callback<Uuid>) -> impl IntoView {
    let post_id = post.id;
    view! {
        <div class="post">
            <div class="post-meta">
                <strong>{post.author_display_name.clone()}</strong>
                <span class="post-time">{post.created_at.format("%Y-%m-%d %H:%M UTC").to_string()}</span>
            </div>
            <div class="post-body" inner_html=post.body_html.clone()></div>
            <button class="reply-button" on:click=move |_| on_reply.run(post_id)>"Antworten"</button>
        </div>
    }
}
