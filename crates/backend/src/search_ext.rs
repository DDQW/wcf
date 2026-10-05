//! Search sources of modules added in later phases.

use shared::search::SearchKind;
use shared::AuthUser;
use sqlx::{Postgres, QueryBuilder};

pub fn kind_query<'a>(
    kind: SearchKind,
    _select: &str,
    query: &'a str,
    viewer: Option<&AuthUser>,
    mut qb: QueryBuilder<'a, Postgres>,
) -> Option<QueryBuilder<'a, Postgres>> {
    match kind {
        SearchKind::File => {
            qb.push(
                "FROM files f JOIN users a ON a.id = f.owner_id, plainto_tsquery('simple', ",
            );
            qb.push_bind(query);
            qb.push(") q WHERE f.deleted_at IS NULL AND f.purpose = 'drive' AND (f.search @@ q OR position(lower(");
            qb.push_bind(query);
            qb.push(") in lower(f.name)) > 0) AND (f.visibility = 'public' OR (f.visibility = 'members' AND ");
            qb.push_bind(viewer.is_some());
            qb.push(") OR f.owner_id = ");
            qb.push_bind(viewer.map(|v| v.id));
            qb.push(")");
            Some(qb)
        }
        SearchKind::ChatMessage => {
            let Some(v) = viewer else { return None };
            qb.push(
                "FROM chat_messages m JOIN chat_channels c ON c.id = m.channel_id \
                 JOIN users a ON a.id = m.author_id, websearch_to_tsquery('german', ",
            );
            qb.push_bind(query);
            qb.push(") q WHERE m.search @@ q AND m.deleted_at IS NULL AND c.kind <> 'room' AND (c.kind = 'public' OR EXISTS (SELECT 1 FROM chat_members cm WHERE cm.channel_id = c.id AND cm.user_id = ");
            qb.push_bind(v.id);
            qb.push("))");
            Some(qb)
        }
        _ => None,
    }
}

pub fn hit_select(kind: SearchKind, author: &str) -> String {
    match kind {
        SearchKind::File => format!(
            "f.id, f.name AS title, f.created_at, {author}, f.mime_type || ' · ' ||              pg_size_pretty(f.size_bytes) AS hl, '/files/' || f.id AS link,              CASE WHEN f.has_thumb THEN '/files/' || f.id || '/thumb' END AS thumb,              ts_rank(f.search, q) AS rank"
        ),
        SearchKind::ChatMessage => format!(
            "m.id, CASE WHEN c.kind IN ('direct', 'group') THEN 'Direktnachricht' ELSE '#' || c.name END AS title, \
             m.created_at, {author}, ts_headline('german', m.body_md, q, 'StartSel=\u{1}, StopSel=\u{2}, MaxWords=30, MinWords=10') AS hl, \
             '/chat/' || c.id || '?m=' || m.id AS link, NULL::text AS thumb, ts_rank(m.search, q) AS rank"
        ),
        _ => String::new(),
    }
}
