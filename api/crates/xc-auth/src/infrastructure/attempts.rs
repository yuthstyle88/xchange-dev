//! Attempt counter on `auth_attempts` (OR-7.m #1027).
//!
//! Password login goes, but the device agent's rate limits keep counting attempts per key and
//! 15-minute window in `auth_attempts`. This counter is that table's one writer once
//! `AccountRepository` is deleted; migration 3 keeps the table.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;

/// Counts attempts per `(key, window_start)` in `auth_attempts`.
#[derive(Clone)]
pub struct PgAttemptCounter {
    pool: PgPool,
}

impl PgAttemptCounter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Adds one attempt for `key` in the window starting at `window_start` and answers the
    /// window's count after it (the first attempt answers 1). Concurrent calls never lose a count.
    pub async fn increment(&self, key: &str, window_start: DateTime<Utc>) -> AppResult<i32> {
        let count: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO auth_attempts (key, window_start, count)
            VALUES ($1, $2, 1)
            ON CONFLICT (key, window_start)
            DO UPDATE SET count = auth_attempts.count + 1
            RETURNING count
            "#,
        )
        .bind(key)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(count)
    }

    /// Deletes every window that started before `current_window` and answers how many went.
    /// The window `current_window` itself, and later ones, stay.
    pub async fn purge_before(&self, current_window: DateTime<Utc>) -> AppResult<u64> {
        let result = sqlx::query("DELETE FROM auth_attempts WHERE window_start < $1")
            .bind(current_window)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(result.rows_affected())
    }
}
