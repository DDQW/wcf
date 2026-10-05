//! Markdown → sanitised HTML. Raw HTML in user input is escaped (shown as
//! text), and the output goes through ammonia as defence in depth.
//! `@username` mentions of existing users become profile links.

use std::collections::HashMap;
use std::sync::LazyLock;

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};
use sqlx::PgPool;
use uuid::Uuid;

/// Bump whenever rendering output changes; stored HTML is then rebuilt at
/// startup (see `maintenance`).
pub const RENDER_VERSION: i32 = 1;

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

static SANITIZER: LazyLock<ammonia::Builder<'static>> = LazyLock::new(|| {
    let mut b = ammonia::Builder::default();
    b.add_tags(["input"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        .add_tag_attributes("a", ["class"])
        .add_tag_attributes("code", ["class"])
        .add_tag_attributes("span", ["class"])
        .link_rel(Some("noopener noreferrer nofollow ugc"))
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .url_relative(ammonia::UrlRelative::PassThrough);
    b
});

pub fn sanitize(html: &str) -> String {
    SANITIZER.clean(html).to_string()
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Byte ranges `(start, end)` of `@name` mentions in a text fragment
/// (`start` points at the `@`). E-mail addresses are not matched.
fn scan_mentions(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        let prev_ok = i == 0 || {
            let p = chars[i - 1].1;
            !(p.is_alphanumeric() || p == '_' || p == '@')
        };
        if c == '@' && prev_ok {
            let mut j = i + 1;
            while j < chars.len() && is_name_char(chars[j].1) {
                j += 1;
            }
            // Trailing punctuation belongs to the sentence, not the name.
            while j > i + 1 && matches!(chars[j - 1].1, '.' | '-') {
                j -= 1;
            }
            let len = j - i - 1;
            if (shared::util::USERNAME_MIN..=shared::util::USERNAME_MAX).contains(&len) {
                let end = chars.get(j).map(|(p, _)| *p).unwrap_or(text.len());
                out.push((pos, end));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Lowercased usernames that are @mentioned outside code and links.
pub fn mention_candidates(md: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut in_code = 0usize;
    let mut in_link = 0usize;
    for ev in Parser::new_ext(md, options()) {
        match ev {
            Event::Start(Tag::CodeBlock(_)) => in_code += 1,
            Event::End(TagEnd::CodeBlock) => in_code = in_code.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => in_link += 1,
            Event::End(TagEnd::Link) => in_link = in_link.saturating_sub(1),
            Event::Text(t) if in_code == 0 && in_link == 0 => {
                for (s, e) in scan_mentions(&t) {
                    let name = t[s + 1..e].to_lowercase();
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            _ => {}
        }
    }
    names
}

/// Renders markdown. `mentions` maps lowercased username → canonical
/// username for users that exist; other `@words` stay plain text.
pub fn render_with(md: &str, mentions: &HashMap<String, String>) -> String {
    let mut events: Vec<Event> = Vec::new();
    let mut in_code = 0usize;
    let mut in_link = 0usize;
    for ev in Parser::new_ext(md, options()) {
        match ev {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code += 1;
                events.push(ev);
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = in_code.saturating_sub(1);
                events.push(ev);
            }
            Event::Start(Tag::Link { .. }) => {
                in_link += 1;
                events.push(ev);
            }
            Event::End(TagEnd::Link) => {
                in_link = in_link.saturating_sub(1);
                events.push(ev);
            }
            // Never pass raw HTML through — show it as text instead.
            Event::Html(h) | Event::InlineHtml(h) => events.push(Event::Text(h)),
            Event::Text(t) if in_code == 0 && in_link == 0 && !mentions.is_empty() => {
                let ranges = scan_mentions(&t);
                if ranges.is_empty() {
                    events.push(Event::Text(t));
                    continue;
                }
                let mut last = 0;
                for (s, e) in ranges {
                    let name = t[s + 1..e].to_lowercase();
                    let Some(canonical) = mentions.get(&name) else {
                        continue;
                    };
                    if s > last {
                        events.push(Event::Text(CowStr::from(t[last..s].to_string())));
                    }
                    let link = format!(
                        "<a class=\"mention\" href=\"/u/{canonical}\">@{canonical}</a>"
                    );
                    events.push(Event::InlineHtml(CowStr::from(link)));
                    last = e;
                }
                if last < t.len() {
                    events.push(Event::Text(CowStr::from(t[last..].to_string())));
                }
            }
            other => events.push(other),
        }
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    sanitize(&out)
}

/// Renders without mention resolution.
pub fn render(md: &str) -> String {
    render_with(md, &HashMap::new())
}

/// Result of rendering with mention lookup.
pub struct Rendered {
    pub html: String,
    /// Ids of mentioned users that exist (and are active).
    pub mentioned: Vec<Uuid>,
}

/// Renders markdown, resolving @mentions against the users table.
pub async fn render_mentions(db: &PgPool, md: &str) -> Result<Rendered, sqlx::Error> {
    let candidates = mention_candidates(md);
    if candidates.is_empty() {
        return Ok(Rendered {
            html: render(md),
            mentioned: Vec::new(),
        });
    }
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, username::text FROM users \
         WHERE lower(username::text) = ANY($1) AND is_active",
    )
    .bind(&candidates)
    .fetch_all(db)
    .await?;
    let map: HashMap<String, String> = rows
        .iter()
        .map(|(_, name)| (name.to_lowercase(), name.clone()))
        .collect();
    Ok(Rendered {
        html: render_with(md, &map),
        mentioned: rows.into_iter().map(|(id, _)| id).collect(),
    })
}

/// Plain text (for excerpts, notifications and previews).
pub fn plain_text(md: &str) -> String {
    let mut out = String::new();
    for ev in Parser::new_ext(md, options()) {
        match ev {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock) => {
                out.push(' ')
            }
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn excerpt(md: &str, max_chars: usize) -> String {
    shared::util::truncate_chars(&plain_text(md), max_chars)
}

/// Escapes text for safe inclusion in HTML.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Turns a `ts_headline` result that used `\u{1}`/`\u{2}` as highlight
/// markers into safe HTML with `<mark>` tags.
pub fn headline_to_html(raw: &str) -> String {
    escape_html(raw).replace('\u{1}', "<mark>").replace('\u{2}', "</mark>")
}

/// URL slug with a short unique suffix.
pub fn slugify(input: &str, unique: &Uuid) -> String {
    let base = shared::util::slug_base(input);
    let suffix = unique.simple().to_string();
    let short = &suffix[suffix.len() - 6..];
    if base.is_empty() {
        short.to_string()
    } else {
        format!("{base}-{short}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_raw_html() {
        let html = render("Hallo <script>alert(1)</script> **Welt**");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<strong>Welt</strong>"));
    }

    #[test]
    fn strips_javascript_links() {
        let html = render("[klick](javascript:alert(1))");
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn finds_mentions_but_not_emails_or_code() {
        let names = mention_candidates("Hi @Anna und @bob. Mail: x@y.de `@code` @ab");
        assert_eq!(names, vec!["anna", "bob"]);
    }

    #[test]
    fn links_known_mentions() {
        let mut map = HashMap::new();
        map.insert("anna".to_string(), "Anna".to_string());
        let html = render_with("Hallo @anna!", &map);
        assert!(html.contains(r#"<a class="mention" href="/u/Anna""#), "{html}");
        let html = render_with("Hallo @nobody!", &map);
        assert!(!html.contains("mention"));
    }

    #[test]
    fn task_lists_and_tables_survive_sanitizing() {
        let html = render("- [x] fertig\n\n| a | b |\n|---|---|\n| 1 | 2 |");
        assert!(html.contains("checkbox"));
        assert!(html.contains("<table>"));
    }

    #[test]
    fn plain_text_and_excerpt() {
        assert_eq!(plain_text("# Titel\n\nText *fett*"), "Titel Text fett");
        assert_eq!(excerpt("eins zwei drei vier", 9), "eins zwe…");
    }

    #[test]
    fn headline_is_escaped() {
        assert_eq!(headline_to_html("<b>\u{1}x\u{2}"), "&lt;b&gt;<mark>x</mark>");
    }
}
