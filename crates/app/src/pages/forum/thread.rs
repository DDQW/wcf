//! Rendering of thread blocks (a top-level post plus its reply tree).

use std::collections::{HashMap, HashSet};

use leptos::html;
use leptos::prelude::*;
use shared::forum::{PostRevision, PostView, ThreadBlock};
use shared::{Cursor, SubjectType};
use uuid::Uuid;

use super::topic::TopicCtx;
use crate::api::forum::{
    delete_post, edit_post, list_replies, post_revisions, post_source, restore_post,
};
use crate::components::common::{confirm, copy_to_clipboard, origin};
use crate::components::{
    Avatar, Icon, MarkdownEditor, Modal, ReactionBar, ReportButton, RoleBadge, Spinner, TimeAgo,
};
use crate::state::{use_session, use_toasts};

/// Blocks with more replies than this start collapsed (unless they contain
/// new or targeted posts).
pub const COLLAPSE_THRESHOLD: usize = 4;
const MAX_INDENT: usize = 5;

/// Orders replies as a tree (depth-first, siblings chronologically).
/// Replies whose parent is not loaded hang directly under the root.
pub fn order_replies(root_id: Uuid, replies: &[PostView]) -> Vec<(PostView, usize)> {
    let ids: HashSet<Uuid> = replies.iter().map(|p| p.id).collect();
    let mut children: HashMap<Uuid, Vec<&PostView>> = HashMap::new();
    for r in replies {
        let parent = r.parent_post_id.filter(|p| ids.contains(p)).unwrap_or(root_id);
        children.entry(parent).or_default().push(r);
    }
    for list in children.values_mut() {
        list.sort_by_key(|p| (p.created_at, p.id));
    }
    let mut out = Vec::with_capacity(replies.len());
    let mut stack: Vec<(Uuid, usize)> = children
        .get(&root_id)
        .map(|v| v.iter().rev().map(|p| (p.id, 1)).collect())
        .unwrap_or_default();
    let by_id: HashMap<Uuid, &PostView> = replies.iter().map(|p| (p.id, p)).collect();
    while let Some((id, depth)) = stack.pop() {
        if let Some(p) = by_id.get(&id) {
            out.push(((*p).clone(), depth.min(MAX_INDENT)));
        }
        if let Some(kids) = children.get(&id) {
            for k in kids.iter().rev() {
                stack.push((k.id, depth + 1));
            }
        }
    }
    out
}

#[component]
pub fn ThreadBlockView(block: ArcRwSignal<ThreadBlock>) -> impl IntoView {
    let ctx = expect_context::<TopicCtx>();
    let session = use_session();
    let toasts = use_toasts();
    let initial = block.get_untracked();
    let root_id = initial.root.id;
    let me = session.user_id();
    let has_interest = initial.replies.iter().any(|p| {
        ctx.highlight.get_untracked() == Some(p.id)
            || ctx.last_read.is_some_and(|t| p.created_at > t && Some(p.author.id) != me)
    });
    let collapsed = RwSignal::new(initial.replies.len() > COLLAPSE_THRESHOLD && !has_interest);
    let loading_more = RwSignal::new(false);
    let block_for_hl = block.clone();
    Effect::new(move |_| {
        if let Some(h) = ctx.highlight.get() {
            if block_for_hl.with(|b| b.replies.iter().any(|p| p.id == h)) {
                collapsed.set(false);
            }
        }
    });

    let block_for_root = block.clone();
    let root = Memo::new(move |_| block_for_root.with(|b| b.root.clone()));
    let block_for_replies = block.clone();
    let ordered = Memo::new(move |_| block_for_replies.with(|b| order_replies(b.root.id, &b.replies)));
    let block_for_counts = block.clone();
    let counts = Memo::new(move |_| block_for_counts.with(|b| (b.replies.len() as i64, b.reply_total)));

    let block_for_more = block.clone();
    let load_more = move |_| {
        let block = block_for_more.clone();
        let after = block.with_untracked(|b| {
            b.replies
                .iter()
                .max_by_key(|p| (p.created_at, p.id))
                .map(|p| Cursor::new(p.created_at, p.id).encode())
        });
        loading_more.set(true);
        leptos::task::spawn_local(async move {
            match list_replies(root_id, after).await {
                Ok(page) => block.update(|b| {
                    let known: HashSet<Uuid> = b.replies.iter().map(|p| p.id).collect();
                    b.replies.extend(page.replies.into_iter().filter(|p| !known.contains(&p.id)));
                    if page.next_cursor.is_none() {
                        b.reply_total = b.replies.len() as i64;
                    }
                }),
                Err(e) => toasts.error(e.to_string()),
            }
            loading_more.set(false);
        });
    };

    let summary = {
        let block = block.clone();
        move || {
            block.with(|b| {
                let mut names: Vec<String> = Vec::new();
                for p in &b.replies {
                    if !names.contains(&p.author.display_name) {
                        names.push(p.author.display_name.clone());
                    }
                }
                let shown: Vec<String> = names.iter().take(3).cloned().collect();
                let more = names.len().saturating_sub(3);
                let mut s = format!("{} Antworten von {}", b.reply_total, shown.join(", "));
                if more > 0 {
                    s.push_str(&format!(" und {more} weiteren"));
                }
                s
            })
        }
    };

    view! {
        <div class="thread-block card" id=format!("block-{root_id}")>
            {move || view! { <PostCard post=root.get() is_root=true/> }}
            <Show when=move || { counts.get().1 > 0 }>
                <button class="collapse-toggle" on:click=move |_| collapsed.update(|c| *c = !*c)>
                    {move || view! { <Icon name=if collapsed.get() { "chevron-right" } else { "chevron-down" } size=16/> }}
                    {let summary = summary.clone(); move || if collapsed.get() { summary() } else { "Antworten einklappen".to_string() }}
                </button>
            </Show>
            <div class="replies" class:hidden=move || collapsed.get()>
                <For
                    each=move || ordered.get()
                    key=|(p, depth)| (p.id, p.edit_count, p.is_deleted, *depth, p.body_html.len())
                    children=move |(p, depth)| view! {
                        <div class="reply" style=format!("--depth:{}", depth.saturating_sub(1))>
                            <PostCard post=p is_root=false/>
                        </div>
                    }
                />
                <Show when=move || { let (loaded, total) = counts.get(); loaded < total }>
                    <button class="btn btn-sm btn-ghost more-replies" disabled=move || loading_more.get() on:click=load_more.clone()>
                        {move || { let (loaded, total) = counts.get(); format!("Weitere {} Antworten laden", total - loaded) }}
                    </button>
                </Show>
            </div>
        </div>
    }
}

#[component]
pub fn PostCard(post: PostView, is_root: bool) -> impl IntoView {
    let ctx = expect_context::<TopicCtx>();
    let session = use_session();
    let toasts = use_toasts();
    let post_id = post.id;
    let author_id = post.author.id;
    let is_first_post = ctx.first_post_id == Some(post_id);
    let is_new = ctx.last_read.is_some_and(|t| post.created_at > t) && session.user_id() != Some(author_id);
    let highlighted = move || ctx.highlight.get() == Some(post_id);
    let editing = RwSignal::new(false);
    let edit_body = RwSignal::new(String::new());
    let revisions_open = RwSignal::new(false);
    let revisions = RwSignal::new(None::<Vec<PostRevision>>);

    let deleted = post.is_deleted;
    let can_edit = move || {
        !deleted
            && session.user.with(|u| {
                u.as_ref()
                    .is_some_and(|u| u.is_mod() || (u.id == author_id && !ctx.detail.with(|d| d.is_locked)))
            })
    };
    let can_delete = move || session.user.with(|u| u.as_ref().is_some_and(|u| u.is_mod() || u.id == author_id)) && !deleted;
    let can_reply = move || ctx.detail.with(|d| d.can_reply) && !deleted;

    let start_edit = move |_| {
        leptos::task::spawn_local(async move {
            match post_source(post_id).await {
                Ok(src) => {
                    edit_body.set(src);
                    editing.set(true);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let save_edit = Callback::new(move |_| {
        let body = edit_body.get_untracked();
        leptos::task::spawn_local(async move {
            match edit_post(post_id, body).await {
                Ok(p) => {
                    editing.set(false);
                    ctx.apply_post.run(p);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    });
    let remove = move |_| {
        let msg = if is_first_post { "Das ganze Thema löschen?" } else { "Diesen Beitrag löschen?" };
        if !confirm(msg) {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_post(post_id).await {
                Ok(()) => {
                    if is_first_post {
                        ctx.on_topic_deleted.run(());
                    } else {
                        ctx.mark_deleted.run(post_id);
                        toasts.success("Beitrag gelöscht.");
                    }
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let restore = move |_| {
        leptos::task::spawn_local(async move {
            match restore_post(post_id).await {
                Ok(p) => ctx.apply_post.run(p),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let reply_post = post.clone();
    let reply = move |_| ctx.start_reply.run((reply_post.clone(), false));
    let quote_post = post.clone();
    let quote = move |_| ctx.start_reply.run((quote_post.clone(), true));
    let topic_id = post.topic_id;
    let share = move |_| {
        copy_to_clipboard(&format!("{}/forum/t/{topic_id}?post={post_id}", origin()));
        toasts.success("Link kopiert.");
    };
    let show_revisions = move |_| {
        revisions_open.set(true);
        if revisions.get_untracked().is_none() {
            leptos::task::spawn_local(async move {
                match post_revisions(post_id).await {
                    Ok(r) => revisions.set(Some(r)),
                    Err(e) => toasts.error(e.to_string()),
                }
            });
        }
    };
    let node = NodeRef::<html::Article>::new();

    view! {
        <article class="post" class:post-root=is_root class:is-new=is_new class:highlight=highlighted
            class:deleted=deleted id=format!("post-{post_id}") node_ref=node>
            <header class="post-head">
                <a href=post.author.profile_url()><Avatar user=post.author.clone() size=if is_root { 40 } else { 32 } presence=true/></a>
                <div class="post-author">
                    <a class="user-link" href=post.author.profile_url()>{post.author.display_name.clone()}</a>
                    <RoleBadge role=post.author.role/>
                    {(!is_root && post.depth > 1).then(|| post.reply_to.clone().map(|name| view! {
                        <span class="reply-to muted small"><Icon name="corner-down-right" size=12/>{name}</span>
                    })).flatten()}
                </div>
                <div class="post-time muted small">
                    <a href=format!("/forum/t/{topic_id}?post={post_id}")><TimeAgo ts=post.created_at/></a>
                    {(post.edit_count > 0).then(|| view! {
                        <button class="link-btn small" title="Versionen anzeigen" on:click=show_revisions>
                            <Icon name="edit" size=12/>"bearbeitet"
                        </button>
                    })}
                    {is_new.then(|| view! { <span class="badge badge-new">"neu"</span> })}
                </div>
            </header>
            {move || if editing.get() {
                view! {
                    <div class="post-edit stack-sm">
                        <MarkdownEditor value=edit_body rows=8 on_submit=save_edit autofocus=true/>
                        <div class="row gap end">
                            <button class="btn btn-sm" on:click=move |_| editing.set(false)>"Abbrechen"</button>
                            <button class="btn btn-sm btn-primary" on:click=move |_| save_edit.run(())>"Speichern"</button>
                        </div>
                    </div>
                }.into_any()
            } else if deleted && post.body_html.is_empty() {
                view! { <p class="post-deleted muted"><Icon name="trash" size=14/>"Dieser Beitrag wurde gelöscht."</p> }.into_any()
            } else {
                view! {
                    {deleted.then(|| view! { <p class="post-deleted muted small"><Icon name="trash" size=14/>"Gelöscht – nur für Moderation sichtbar"</p> })}
                    <div class="post-body prose" inner_html=post.body_html.clone()></div>
                }.into_any()
            }}
            <footer class="post-foot">
                {(!deleted).then(|| view! { <ReactionBar subject_type=SubjectType::ForumPost subject_id=post_id reactions=post.reactions.clone()/> })}
                <div class="post-actions">
                    <Show when=can_reply>
                        <button class="action-btn" on:click=reply.clone()><Icon name="reply" size=15/>"Antworten"</button>
                        <button class="action-btn" on:click=quote.clone()><Icon name="quote" size=15/>"Zitieren"</button>
                    </Show>
                    <button class="action-btn" title="Link kopieren" on:click=share><Icon name="link" size=15/></button>
                    <Show when=can_edit>
                        <button class="action-btn" title="Bearbeiten" on:click=start_edit><Icon name="edit" size=15/></button>
                    </Show>
                    <Show when=can_delete>
                        <button class="action-btn" title="Löschen" on:click=remove><Icon name="trash" size=15/></button>
                    </Show>
                    <Show when=move || deleted && session.is_mod()>
                        <button class="action-btn" title="Wiederherstellen" on:click=restore><Icon name="rotate-cw" size=15/>"Wiederherstellen"</button>
                    </Show>
                    <Show when=move || !deleted && session.user_id().is_some_and(|me| me != author_id)>
                        <ReportButton subject_type=SubjectType::ForumPost subject_id=post_id compact=true/>
                    </Show>
                </div>
            </footer>
            <Modal open=revisions_open title="Frühere Versionen" wide=true>
                {move || match revisions.get() {
                    None => view! { <Spinner/> }.into_any(),
                    Some(list) if list.is_empty() => view! { <p class="muted">"Keine früheren Versionen."</p> }.into_any(),
                    Some(list) => view! {
                        <div class="stack">
                            {list.into_iter().map(|r| view! {
                                <div class="revision">
                                    <div class="muted small">"Vor der Änderung von " {r.editor.display_name.clone()} " · " <TimeAgo ts=r.created_at absolute=true/></div>
                                    <div class="prose" inner_html=r.body_html.clone()></div>
                                </div>
                            }).collect_view()}
                        </div>
                    }.into_any(),
                }}
            </Modal>
        </article>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use shared::{Role, UserSummary};

    fn post(id: u128, parent: Option<u128>, root: u128, minutes: i64) -> PostView {
        PostView {
            id: Uuid::from_u128(id),
            topic_id: Uuid::nil(),
            author: UserSummary {
                id: Uuid::nil(),
                username: "a".into(),
                display_name: "A".into(),
                avatar_url: None,
                role: Role::Member,
            },
            parent_post_id: parent.map(Uuid::from_u128),
            thread_root_id: Uuid::from_u128(root),
            depth: 0,
            body_html: String::new(),
            created_at: Utc::now() + Duration::minutes(minutes),
            edited_at: None,
            edit_count: 0,
            is_deleted: false,
            reactions: vec![],
            reply_to: None,
        }
    }

    #[test]
    fn replies_are_tree_ordered() {
        // root 1; 2 replies to 1 (t=1), 3 replies to 1 (t=2), 4 replies to 2 (t=3)
        let replies = vec![post(2, Some(1), 1, 1), post(3, Some(1), 1, 2), post(4, Some(2), 1, 3)];
        let ordered: Vec<(u128, usize)> = order_replies(Uuid::from_u128(1), &replies)
            .into_iter()
            .map(|(p, d)| (p.id.as_u128(), d))
            .collect();
        assert_eq!(ordered, vec![(2, 1), (4, 2), (3, 1)]);
    }

    #[test]
    fn orphans_hang_under_root() {
        let replies = vec![post(5, Some(99), 1, 1)];
        let ordered = order_replies(Uuid::from_u128(1), &replies);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].1, 1);
    }
}
