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
        _ => None,
    }
}

pub fn hit_select(kind: SearchKind, author: &str) -> String {
    match kind {
        SearchKind::File => format!(
            "f.id, f.name AS title, f.created_at, {author}, f.mime_type || ' · ' ||              pg_size_pretty(f.size_bytes) AS hl, '/files/' || f.id AS link,              CASE WHEN f.has_thumb THEN '/files/' || f.id || '/thumb' END AS thumb,              ts_rank(f.search, q) AS rank"
        ),
        _ => String::new(),
    }
}
