//! Global full-text search across modules (PostgreSQL `tsvector`).

use shared::search::{SearchHit, SearchKind, SearchResults};
use shared::AuthUser;
use sqlx::{Postgres, QueryBuilder, Row};

use crate::error::Result;
use crate::markdown::headline_to_html;
use crate::state::AppState;
use crate::users::{user_cols, user_from};

/// `ts_headline` options; \u{1}/\u{2} are replaced by `<mark>` after escaping.
const HL_OPTS: &str = "StartSel=\u{1}, StopSel=\u{2}, MaxWords=30, MinWords=12, \
                       MaxFragments=2, FragmentDelimiter=\" … \"";
const HL_TITLE: &str = "StartSel=\u{1}, StopSel=\u{2}, HighlightAll=true";

fn push_cat_visible(qb: &mut QueryBuilder<'_, Postgres>, viewer: Option<&AuthUser>) {
    qb.push(" (c.read_access = 'public' OR (c.read_access = 'members' AND ");
    qb.push_bind(viewer.is_some());
    qb.push(") OR ");
    qb.push_bind(viewer.is_some_and(|v| v.is_mod()));
    qb.push(") ");
}

/// Builds the FROM/WHERE part for one kind; `select` decides what to fetch.
fn kind_query<'a>(
    kind: SearchKind,
    select: &str,
    query: &'a str,
    viewer: Option<&AuthUser>,
) -> Option<QueryBuilder<'a, Postgres>> {
    let mut qb = QueryBuilder::new(format!("SELECT {select} "));
    match kind {
        SearchKind::ForumTopic => {
            qb.push(
                "FROM forum_topics t JOIN forum_categories c ON c.id = t.category_id \
                 JOIN users a ON a.id = t.author_id, websearch_to_tsquery('german', ",
            );
            qb.push_bind(query);
            qb.push(") q WHERE t.search @@ q AND NOT t.is_deleted AND ");
            push_cat_visible(&mut qb, viewer);
        }
        SearchKind::ForumPost => {
            qb.push(
                "FROM forum_posts p JOIN forum_topics t ON t.id = p.topic_id \
                 JOIN forum_categories c ON c.id = t.category_id \
                 JOIN users a ON a.id = p.author_id, websearch_to_tsquery('german', ",
            );
            qb.push_bind(query);
            qb.push(") q WHERE p.search @@ q AND NOT p.is_deleted AND NOT t.is_deleted AND ");
            push_cat_visible(&mut qb, viewer);
        }
        SearchKind::User => {
            qb.push("FROM users a WHERE a.is_active AND (a.search @@ plainto_tsquery('simple', ");
            qb.push_bind(query);
            qb.push(") OR position(lower(");
            qb.push_bind(query);
            qb.push(") in lower(a.display_name)) > 0 OR position(lower(");
            qb.push_bind(query);
            qb.push(") in lower(a.username::text)) > 0)");
        }
        other => return crate::search_ext::kind_query(other, select, query, viewer, qb),
    }
    Some(qb)
}

fn hit_select(kind: SearchKind) -> String {
    let author = user_cols("a", "author");
    match kind {
        SearchKind::ForumTopic => format!(
            "t.id, t.title, t.created_at, {author}, \
             ts_headline('german', t.title, q, '{HL_TITLE}') AS hl, \
             '/forum/t/' || t.id AS link, NULL::text AS thumb, ts_rank(t.search, q) AS rank"
        ),
        SearchKind::ForumPost => format!(
            "p.id, t.title, p.created_at, {author}, \
             ts_headline('german', p.body_md, q, '{HL_OPTS}') AS hl, \
             '/forum/t/' || t.id || '?post=' || p.id AS link, NULL::text AS thumb, \
             ts_rank(p.search, q) AS rank"
        ),
        SearchKind::User => format!(
            "a.id, a.display_name AS title, a.created_at, {author}, \
             '@' || a.username::text AS hl, '/u/' || a.username::text AS link, NULL::text AS thumb, \
             1.0::real AS rank"
        ),
        other => crate::search_ext::hit_select(other, &author),
    }
}

pub async fn search(
    state: &AppState,
    viewer: Option<&AuthUser>,
    query: &str,
    kind: Option<SearchKind>,
) -> Result<SearchResults> {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Ok(SearchResults::default());
    }
    let modules = state.settings().modules;
    let enabled = |k: SearchKind| match k {
        SearchKind::ForumTopic | SearchKind::ForumPost => modules.forum,
        SearchKind::BlogPost => modules.blogs,
        SearchKind::ChatMessage => modules.chat,
        SearchKind::File => modules.files,
        SearchKind::GalleryImage => modules.gallery,
        SearchKind::CmsEntry => modules.cms,
        SearchKind::User => true,
    };

    let mut results = SearchResults::default();
    for k in SearchKind::ALL.into_iter().filter(|k| enabled(*k)) {
        let Some(mut qb) = kind_query(k, "count(*) AS n", query, viewer) else { continue };
        let n: i64 = match qb.build().fetch_one(&state.db).await {
            Ok(r) => r.try_get("n")?,
            Err(e) => {
                tracing::warn!(error = %e, kind = k.as_str(), "search count failed");
                continue;
            }
        };
        if n > 0 {
            results.counts.push((k, n));
        }
    }

    let kinds: Vec<SearchKind> = match kind {
        Some(k) => vec![k],
        None => results.counts.iter().map(|(k, _)| *k).collect(),
    };
    let per_kind = if kind.is_some() { 50 } else { 5 };
    for k in kinds {
        let Some(mut qb) = kind_query(k, &hit_select(k), query, viewer) else { continue };
        qb.push(" ORDER BY rank DESC, 3 DESC LIMIT ");
        qb.push_bind(per_kind as i64);
        let rows = match qb.build().fetch_all(&state.db).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, kind = k.as_str(), "search failed");
                continue;
            }
        };
        for r in rows {
            let hl: Option<String> = r.try_get("hl")?;
            results.hits.push(SearchHit {
                kind: k,
                title: r.try_get("title")?,
                snippet_html: headline_to_html(&hl.unwrap_or_default()),
                link: r.try_get("link")?,
                author: user_from(&r, "author").ok(),
                created_at: r.try_get("created_at")?,
                thumb_url: r.try_get("thumb")?,
            });
        }
    }
    Ok(results)
}
