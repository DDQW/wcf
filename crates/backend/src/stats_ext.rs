//! Admin statistics of modules added in later phases.

use shared::AdminStats;
use sqlx::PgPool;

use crate::error::Result;

pub const EXTRA_SQL: &str = "0";

pub async fn fill(_db: &PgPool, _stats: &mut AdminStats) -> Result<()> {
    Ok(())
}
