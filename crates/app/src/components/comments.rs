use leptos::prelude::*;
use shared::blog::CommentView;
use shared::SubjectType;
use uuid::Uuid;

use crate::api::blog::{add_comment, comment_source, delete_comment, edit_comment, list_comments};
use crate::components::common::confirm;
use crate::components::{Avatar, Icon, MarkdownEditor, ReactionBar, ReportButton, Spinner, TimeAgo};
use crate::state::{use_session, use_toasts};

/// Threaded comments (two levels: comments and replies) for any subject.
#[component]
pub fn CommentsSection(
    subject_type: SubjectType,
    subject_id: Uuid,
    #[prop(default = true)] allowed: bool,
) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let comments = RwSignal::new(None::<Vec<CommentView>>);
    let body = RwSignal::new(String::new());
    let reply_to = RwSignal::new(None::<Uuid>);
    let sending = RwSignal::new(false);
    let load = move || {
        leptos::task::spawn_local(async move {
            match list_comments(subject_type, subject_id).await {
                Ok(list) => comments.set(Some(list)),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    Effect::new(move |_| load());

    let submit = Callback::new(move |_| {
        if body.with_untracked(|b| b.trim().is_empty()) {
            return;
        }
        sending.set(true);
        let parent = reply_to.get_untracked();
        let text = body.get_untracked();
        leptos::task::spawn_local(async move {
            match add_comment(subject_type, subject_id, parent, text).await {
                Ok(c) => {
                    comments.update(|list| {
                        if let Some(l) = list {
                            l.push(c);
                        }
                    });
                    body.set(String::new());
                    reply_to.set(None);
                }
                Err(e) => toasts.error(e.to_string()),
            }
            sending.set(false);
        });
    });

    let count = move || comments.with(|c| c.as_ref().map(|l| l.iter().filter(|c| !c.deleted).count()).unwrap_or(0));

    view! {
        <section class="comments card" id="comments">
            <h2><Icon name="message-circle"/>"Kommentare " <span class="muted">{count}</span></h2>
            {move || match comments.get() {
                None => view! { <Spinner/> }.into_any(),
                Some(list) => {
                    let roots: Vec<CommentView> = list.iter().filter(|c| c.parent_id.is_none()).cloned().collect();
                    if roots.is_empty() {
                        view! { <p class="muted">"Noch keine Kommentare."</p> }.into_any()
                    } else {
                        view! {
                            <ul class="comment-list">
                                {roots.into_iter().map(|c| {
                                    let replies: Vec<CommentView> = list.iter().filter(|r| r.parent_id == Some(c.id)).cloned().collect();
                                    view! {
                                        <li>
                                            <CommentItem c=c.clone() comments=comments reply_to=reply_to allowed=allowed/>
                                            {(!replies.is_empty()).then(|| view! {
                                                <ul class="comment-replies">
                                                    {replies.into_iter().map(|r| view! {
                                                        <li><CommentItem c=r comments=comments reply_to=reply_to allowed=false/></li>
                                                    }).collect_view()}
                                                </ul>
                                            })}
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }
                }
            }}
            {move || if !allowed {
                view! { <p class="muted small"><Icon name="lock" size=14/>"Kommentare sind deaktiviert."</p> }.into_any()
            } else if !session.logged_in() {
                view! { <p class="muted"><a href="/login">"Melde dich an"</a>", um zu kommentieren."</p> }.into_any()
            } else {
                view! {
                    <div class="comment-composer stack-sm">
                        {move || reply_to.get().map(|_| view! {
                            <div class="reply-target"><Icon name="corner-down-right" size=14/>"Antwort auf einen Kommentar"
                                <button class="link-btn" on:click=move |_| reply_to.set(None)>"(abbrechen)"</button></div>
                        })}
                        <MarkdownEditor value=body rows=3 on_submit=submit placeholder="Dein Kommentar …"/>
                        <div class="row end">
                            <button class="btn btn-primary" disabled=move || sending.get() on:click=move |_| submit.run(())>
                                <Icon name="send" size=16/>"Kommentieren"
                            </button>
                        </div>
                    </div>
                }.into_any()
            }}
        </section>
    }
}

#[component]
fn CommentItem(
    c: CommentView,
    comments: RwSignal<Option<Vec<CommentView>>>,
    reply_to: RwSignal<Option<Uuid>>,
    allowed: bool,
) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let id = c.id;
    let editing = RwSignal::new(false);
    let edit_body = RwSignal::new(String::new());
    let html = RwSignal::new(c.body_html.clone());
    let author_id = c.author.as_ref().map(|a| a.id);
    let start_edit = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(src) = comment_source(id).await {
                edit_body.set(src);
                editing.set(true);
            }
        });
    };
    let save = Callback::new(move |_| {
        let b = edit_body.get_untracked();
        leptos::task::spawn_local(async move {
            match edit_comment(id, b).await {
                Ok(h) => {
                    html.set(h);
                    editing.set(false);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    });
    let remove = move |_| {
        if !confirm("Kommentar löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_comment(id).await {
                Ok(()) => comments.update(|l| {
                    if let Some(l) = l {
                        if let Some(c) = l.iter_mut().find(|c| c.id == id) {
                            c.deleted = true;
                            c.body_html.clear();
                        }
                    }
                }),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <article class="comment" id=format!("comment-{id}")>
            <div class="comment-head">
                {c.author.clone().map(|a| view! {
                    <Avatar user=a.clone() size=28/>
                    <a class="user-link" href=a.profile_url()>{a.display_name.clone()}</a>
                })}
                <span class="muted small"><TimeAgo ts=c.created_at/></span>
                {c.edited_at.map(|_| view! { <span class="muted small">"· bearbeitet"</span> })}
            </div>
            {if c.deleted {
                view! { <p class="muted small comment-body"><Icon name="trash" size=13/>"Kommentar gelöscht"</p> }.into_any()
            } else {
                view! {
                    <Show when=move || editing.get() fallback=move || view! { <div class="prose comment-body" inner_html=move || html.get()></div> }>
                        <div class="comment-body stack-sm">
                            <MarkdownEditor value=edit_body rows=3 on_submit=save/>
                            <div class="row gap end">
                                <button class="btn btn-sm" on:click=move |_| editing.set(false)>"Abbrechen"</button>
                                <button class="btn btn-sm btn-primary" on:click=move |_| save.run(())>"Speichern"</button>
                            </div>
                        </div>
                    </Show>
                    <div class="comment-foot">
                        <ReactionBar subject_type=SubjectType::Comment subject_id=id reactions=c.reactions.clone()/>
                        <div class="post-actions">
                            {(allowed && session.logged_in()).then(|| view! {
                                <button class="action-btn" on:click=move |_| reply_to.set(Some(id))><Icon name="reply" size=14/>"Antworten"</button>
                            })}
                            {c.can_edit.then(|| view! { <button class="action-btn" on:click=start_edit><Icon name="edit" size=14/></button> })}
                            {c.can_delete.then(|| view! { <button class="action-btn" on:click=remove><Icon name="trash" size=14/></button> })}
                            {(session.logged_in() && session.user_id() != author_id).then(|| view! {
                                <ReportButton subject_type=SubjectType::Comment subject_id=id compact=true/>
                            })}
                        </div>
                    </div>
                }.into_any()
            }}
        </article>
    }
}
