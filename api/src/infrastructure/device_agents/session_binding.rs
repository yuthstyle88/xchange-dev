//! A staff session bound to a counter: `user_sessions.counter_agent_id` (xc-counter-app.md §5.4, §8.4, §8.5).
//! `deskBindCounter` writes it; the save path reads it into `SaveExtras.counter_agent_id`.
//! Sign-out deletes the row, so the binding goes with it; `switch_branch` and revoking the counter clear it.
//!
//! CA-1c fills the bodies.

use pos_error::AppResult;
use sqlx::PgPool;

#[derive(Clone)]
pub struct PgSessionBinding {
    pool: PgPool,
}

impl PgSessionBinding {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Sets `counter_agent_id` on the live session row `session_hash` (`user_sessions.hash`). `true` when the
    /// row exists.
    pub async fn bind_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        session_hash: &str,
        agent_id: &str,
    ) -> AppResult<bool> {
        let done =
            sqlx::query("UPDATE user_sessions SET counter_agent_id = $2::uuid WHERE hash = $1")
                .bind(session_hash)
                .bind(agent_id)
                .execute(&mut **tx)
                .await?;
        Ok(done.rows_affected() == 1)
    }

    /// The counter the session `session_hash` is bound to; `None` when unbound or the row is gone.
    pub async fn bound_counter(&self, session_hash: &str) -> AppResult<Option<String>> {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT counter_agent_id::text FROM user_sessions WHERE hash = $1")
                .bind(session_hash)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.flatten())
    }
}
