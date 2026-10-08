//! Reader of branch versions for the v3 view core (§7.1).

use pos_error::AppResult;
use sqlx::PgPool;

use crate::ids::BranchId;

/// Reader of the `branch_versions` table.
#[derive(Clone)]
pub struct BranchVersions {
    pool: PgPool,
}

impl BranchVersions {
    /// Creates a new `BranchVersions` reader with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// `SELECT version FROM branch_versions WHERE branch_id = $1`; 0 when there is no row.
    pub async fn current(&self, branch: &BranchId) -> AppResult<i64> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT version FROM branch_versions WHERE branch_id = $1")
                .bind(branch.as_str())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(v,)| v).unwrap_or(0))
    }
}
