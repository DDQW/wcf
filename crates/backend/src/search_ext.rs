//! Search sources of modules added in later phases.

use shared::search::SearchKind;
use shared::AuthUser;
use sqlx::{Postgres, QueryBuilder};

pub fn kind_query<'a>(
    _kind: SearchKind,
    _select: &str,
    _query: &'a str,
    _viewer: Option<&AuthUser>,
    _qb: QueryBuilder<'a, Postgres>,
) -> Option<QueryBuilder<'a, Postgres>> {
    None
}

pub fn hit_select(_kind: SearchKind, _author: &str) -> String {
    String::new()
}
