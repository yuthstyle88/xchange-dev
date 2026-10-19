//! Attempt counter on `auth_attempts` (OR-7.m #1027).
//!
//! Password login goes, but the device agent's rate limits keep counting attempts per key and
//! 15-minute window in `auth_attempts`. This counter is that table's one writer once
//! `AccountRepository` is deleted; migration 3 keeps the table.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
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
        let _ = (&self.pool, key, window_start);
        todo!("OR-7.m T2: INSERT … ON CONFLICT (key, window_start) DO UPDATE SET count = count + 1 RETURNING count")
    }

    /// Deletes every window that started before `current_window` and answers how many went.
    /// The window `current_window` itself, and later ones, stay.
    pub async fn purge_before(&self, current_window: DateTime<Utc>) -> AppResult<u64> {
        let _ = (&self.pool, current_window);
        todo!("OR-7.m T2: DELETE FROM auth_attempts WHERE window_start < $1")
    }
}
