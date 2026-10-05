use leptos::html;
use leptos::prelude::*;
use shared::UserSummary;

use crate::components::common::{sleep_ms, Avatar};
use crate::components::icon::Icon;
use crate::api::admin::preview_markdown;

#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
enum Fmt {
    Wrap(&'static str, &'static str, &'static str),
    Line(&'static str),
    Code,
    Link,
}

/// Textarea with formatting toolbar, preview tab, @mention suggestions and
/// Ctrl+Enter to submit.
#[component]
pub fn MarkdownEditor(
    value: RwSignal<String>,
    #[prop(optional, into)] placeholder: String,
    #[prop(default = 6)] rows: u32,
    #[prop(optional)] on_submit: Option<Callback<()>>,
    #[prop(optional)] autofocus: bool,
    #[prop(optional)] textarea_ref: Option<NodeRef<html::Textarea>>,
    /// Extra toolbar content (e.g. an upload button).
    #[prop(optional)]
    toolbar_extra: Option<ChildrenFn>,
) -> impl IntoView {
    let node = textarea_ref.unwrap_or_default();
    let preview = RwSignal::new(false);
    let preview_html = RwSignal::new(String::new());
    let preview_loading = RwSignal::new(false);
    let suggestions = RwSignal::new(Vec::<UserSummary>::new());
    let highlighted = RwSignal::new(0usize);
    // UTF-16 offset of the '@' that started the current mention, and query.
    let mention = RwSignal::new(None::<(u32, String)>);

    let apply = move |fmt: Fmt| {
        #[cfg(feature = "hydrate")]
        if let Some(el) = node.get() {
            apply_format(&el, fmt);
            value.set(el.value());
            let _ = el.focus();
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = fmt;
    };

    let show_preview = move |_| {
        preview.set(true);
        preview_loading.set(true);
        let md = value.get_untracked();
        leptos::task::spawn_local(async move {
            let html = preview_markdown(md).await.unwrap_or_else(|e| {
                format!("<p class=\"error\">{}</p>", e)
            });
            preview_html.set(html);
            preview_loading.set(false);
        });
    };

    let choose = move |user: UserSummary| {
        #[cfg(feature = "hydrate")]
        if let (Some(el), Some((start, query))) = (node.get(), mention.get_untracked()) {
            let end = start + 1 + query.encode_utf16().count() as u32;
            splice(&el, start, end, &format!("@{} ", user.username));
            value.set(el.value());
            let _ = el.focus();
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = user;
        mention.set(None);
        suggestions.set(Vec::new());
    };

    let on_input = move |ev: leptos::ev::Event| {
        value.set(event_target_value(&ev));
        #[cfg(feature = "hydrate")]
        if let Some(el) = node.get() {
            match current_mention(&el) {
                Some((start, query)) if !query.is_empty() => {
                    mention.set(Some((start, query.clone())));
                    leptos::task::spawn_local(async move {
                        sleep_ms(150).await;
                        if mention.get_untracked().map(|(_, q)| q) != Some(query.clone()) {
                            return;
                        }
                        if let Ok(list) = crate::api::users::mention_suggest(query).await {
                            highlighted.set(0);
                            suggestions.set(list);
                        }
                    });
                }
                _ => {
                    mention.set(None);
                    suggestions.set(Vec::new());
                }
            }
        }
    };

    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        let list = suggestions.get_untracked();
        if !list.is_empty() {
            match ev.key().as_str() {
                "ArrowDown" => {
                    ev.prevent_default();
                    highlighted.update(|h| *h = (*h + 1) % list.len());
                    return;
                }
                "ArrowUp" => {
                    ev.prevent_default();
                    highlighted.update(|h| *h = (*h + list.len() - 1) % list.len());
                    return;
                }
                "Enter" | "Tab" => {
                    ev.prevent_default();
                    if let Some(u) = list.get(highlighted.get_untracked()).cloned() {
                        choose(u);
                    }
                    return;
                }
                "Escape" => {
                    suggestions.set(Vec::new());
                    mention.set(None);
                    return;
                }
                _ => {}
            }
        }
        if ev.key() == "Enter" && (ev.ctrl_key() || ev.meta_key()) {
            if let Some(cb) = on_submit {
                ev.prevent_default();
                cb.run(());
            }
        }
    };

    let tool = move |icon: &'static str, title: &'static str, fmt: Fmt| {
        view! {
            <button type="button" class="tool-btn" title=title aria-label=title
                on:mousedown=|ev| ev.prevent_default()
                on:click=move |_| apply(fmt)>
                <Icon name=icon size=16/>
            </button>
        }
    };

    view! {
        <div class="md-editor">
            <div class="md-toolbar">
                <div class="md-tabs">
                    <button type="button" class:active=move || !preview.get() on:click=move |_| preview.set(false)>"Schreiben"</button>
                    <button type="button" class:active=move || preview.get() on:click=show_preview>"Vorschau"</button>
                </div>
                <div class="md-tools" class:hidden=move || preview.get()>
                    {tool("bold", "Fett", Fmt::Wrap("**", "**", "fett"))}
                    {tool("italic", "Kursiv", Fmt::Wrap("_", "_", "kursiv"))}
                    {tool("strike", "Durchgestrichen", Fmt::Wrap("~~", "~~", "durchgestrichen"))}
                    {tool("heading", "Überschrift", Fmt::Line("## "))}
                    {tool("quote", "Zitat", Fmt::Line("> "))}
                    {tool("code", "Code", Fmt::Code)}
                    {tool("link", "Link", Fmt::Link)}
                    {tool("list", "Liste", Fmt::Line("- "))}
                    {tool("list-ordered", "Nummerierte Liste", Fmt::Line("1. "))}
                    {toolbar_extra.as_ref().map(|t| t())}
                </div>
            </div>
            <div class="md-body">
                <textarea
                    node_ref=node
                    class:hidden=move || preview.get()
                    rows=rows
                    placeholder=placeholder
                    autofocus=autofocus
                    prop:value=move || value.get()
                    on:input=on_input
                    on:keydown=on_keydown
                    on:blur=move |_| {
                        // Let a click on a suggestion register first.
                        leptos::task::spawn_local(async move {
                            sleep_ms(200).await;
                            suggestions.set(Vec::new());
                        });
                    }
                ></textarea>
                <Show when=move || preview.get()>
                    <div class="md-preview prose">
                        {move || if preview_loading.get() {
                            view! { <p class="muted">"Vorschau wird erstellt …"</p> }.into_any()
                        } else if value.with(|v| v.trim().is_empty()) {
                            view! { <p class="muted">"Nichts zu sehen."</p> }.into_any()
                        } else {
                            view! { <div inner_html=move || preview_html.get()></div> }.into_any()
                        }}
                    </div>
                </Show>
                <Show when=move || !suggestions.with(|s| s.is_empty())>
                    <ul class="mention-list" role="listbox">
                        <For
                            each=move || suggestions.get().into_iter().enumerate()
                            key=|(_, u)| u.id
                            children=move |(i, u)| {
                                let pick = u.clone();
                                view! {
                                    <li
                                        class:active=move || highlighted.get() == i
                                        on:mousedown=move |ev| { ev.prevent_default(); choose(pick.clone()); }
                                    >
                                        <Avatar user=u.clone() size=22/>
                                        <span>{u.display_name.clone()}</span>
                                        <span class="muted">"@"{u.username.clone()}</span>
                                    </li>
                                }
                            }
                        />
                    </ul>
                </Show>
            </div>
            <div class="md-hint muted small">"Markdown wird unterstützt · @name erwähnt jemanden"
                {on_submit.is_some().then_some(" · Strg+Enter sendet")}
            </div>
        </div>
    }
}

/// Replaces the UTF-16 range `[start, end)` of the textarea with `insert`
/// and puts the caret after it.
#[cfg(feature = "hydrate")]
fn splice(el: &web_sys::HtmlTextAreaElement, start: u32, end: u32, insert: &str) {
    let v = js_sys::JsString::from(el.value());
    let len = v.length();
    let before = v.slice(0, start.min(len));
    let after = v.slice(end.min(len), len);
    let new_value = before.concat(&js_sys::JsString::from(insert).into()).concat(&after.into());
    el.set_value(&String::from(new_value));
    let caret = start + insert.encode_utf16().count() as u32;
    let _ = el.set_selection_range(caret, caret);
}

#[cfg(feature = "hydrate")]
fn apply_format(el: &web_sys::HtmlTextAreaElement, fmt: Fmt) {
    let v = js_sys::JsString::from(el.value());
    let len = v.length();
    let start = el.selection_start().ok().flatten().unwrap_or(len).min(len);
    let end = el.selection_end().ok().flatten().unwrap_or(start).min(len).max(start);
    let selected = String::from(v.slice(start, end));
    let (before, after, placeholder) = match fmt {
        Fmt::Wrap(b, a, p) => (b.to_string(), a.to_string(), p),
        Fmt::Code if selected.contains('\n') => ("```\n".to_string(), "\n```".to_string(), "code"),
        Fmt::Code => ("`".to_string(), "`".to_string(), "code"),
        Fmt::Link => ("[".to_string(), "](https://)".to_string(), "Linktext"),
        Fmt::Line(prefix) => {
            // Insert the prefix at the start of the current line.
            let line_start = if start == 0 {
                0
            } else {
                let idx = v.last_index_of("\n", (start - 1) as i32);
                if idx < 0 { 0 } else { idx as u32 + 1 }
            };
            let before = v.slice(0, line_start);
            let rest = v.slice(line_start, len);
            let new_value = before.concat(&js_sys::JsString::from(prefix).into()).concat(&rest.into());
            el.set_value(&String::from(new_value));
            let shift = prefix.encode_utf16().count() as u32;
            let _ = el.set_selection_range(start + shift, end + shift);
            return;
        }
    };
    let inner = if selected.is_empty() { placeholder.to_string() } else { selected };
    let insert = format!("{before}{inner}{after}");
    let head = v.slice(0, start);
    let tail = v.slice(end, len);
    let new_value = head.concat(&js_sys::JsString::from(insert.as_str()).into()).concat(&tail.into());
    el.set_value(&String::from(new_value));
    let sel_start = start + before.encode_utf16().count() as u32;
    let sel_end = sel_start + inner.encode_utf16().count() as u32;
    let _ = el.set_selection_range(sel_start, sel_end);
}

/// The `@query` directly before the caret, as (UTF-16 offset of '@', query).
#[cfg(feature = "hydrate")]
fn current_mention(el: &web_sys::HtmlTextAreaElement) -> Option<(u32, String)> {
    let v = js_sys::JsString::from(el.value());
    let caret = el.selection_start().ok().flatten()?;
    let head = String::from(v.slice(0, caret));
    let at = head.rfind('@')?;
    let query = &head[at + 1..];
    if query.len() > 32 || !query.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return None;
    }
    let prev_ok = head[..at].chars().last().is_none_or(|c| c.is_whitespace() || c == '(');
    if !prev_ok {
        return None;
    }
    let at_utf16 = head[..at].encode_utf16().count() as u32;
    Some((at_utf16, query.to_string()))
}
