//! Staff directory service (§3.2).
//!
//! Maps user IDs to user names from the `users` table once per call.

use std::collections::HashMap;

use pos_error::{AppError, AppResult};
use sqlx::PgPool;

#[derive(Clone)]
pub struct StaffDirectory {
    pool: PgPool,
}

impl StaffDirectory {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Looks up a single user's name.
    pub async fn name(&self, user_id: &str) -> AppResult<Option<String>> {
        let name: Option<String> = sqlx::query_scalar("SELECT name FROM users WHERE id::text = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(name)
    }

    /// Looks up names for a batch of user IDs once per call.
    pub async fn names(&self, user_ids: &[&str]) -> AppResult<HashMap<String, String>> {
        if user_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<String> = user_ids.iter().map(|s| s.to_string()).collect();
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id::text, name FROM users WHERE id::text = ANY($1)")
                .bind(&ids)
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?;

        Ok(rows.into_iter().collect())
    }
}
