use pulldown_cmark::{html, Options, Parser};

/// Renders markdown to HTML once at write time; the result is cached in
/// `forum_posts.body_html` so reads never re-render.
pub fn render(body_md: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(body_md, options);
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);
    html_out
}

/// Turns a title into a URL-safe slug, with a short suffix to keep it
/// unique enough without a DB round-trip before the row is inserted.
pub fn slugify(input: &str, unique_suffix: &uuid::Uuid) -> String {
    let base: String = input
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let base = base
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let suffix = unique_suffix.simple().to_string();
    let short_suffix = &suffix[..8];
    if base.is_empty() {
        short_suffix.to_string()
    } else {
        format!("{base}-{short_suffix}")
    }
}
