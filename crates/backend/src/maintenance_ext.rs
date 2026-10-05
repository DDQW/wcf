//! Re-render hooks of modules added in later phases.

use sqlx::PgPool;

use crate::error::Result;

pub async fn rerender(_db: &PgPool) -> Result<()> {
    Ok(())
}
