//! K6/P13: expired `user_sessions` rows are deleted on the purge timer (`api/src/bootstrap/purge_timers.rs`).
//! Before this, every delete was per token (refresh, logout); nothing deleted by `expires`.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;

#[derive(Clone)]
pub struct PgExpiredSessions {
    pool: PgPool,
}

impl PgExpiredSessions {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Deletes every session whose `expires` is strictly before `now` and answers how many went. A session that
    /// expires exactly at `now`, or later, stays.
    pub async fn purge_before(&self, now: DateTime<Utc>) -> AppResult<u64> {
        let result = sqlx::query("DELETE FROM user_sessions WHERE expires < $1")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(result.rows_affected())
    }
}
