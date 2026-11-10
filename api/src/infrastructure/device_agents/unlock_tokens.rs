//! The one-time code that lets a locked counter leave kiosk mode (xc-counter-app.md §8.8, xchange#6569).
//! `counterUnlockIssue` writes a row for a manager at the desk; `POST /api/device-agent/kiosk-exit` uses it up.
//!
//! The code has the pairing code's shape (`XXXXX-XXXXX`, Crockford base32) and is stored only as its hash. The
//! server is the only judge: a code opens one counter, once, within [`TTL_SECS`].

use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;

use crate::infrastructure::paired_devices::secret::DeviceSecret;

/// How long a code is good for, in seconds (owner decision, 2026-11-09).
pub const TTL_SECS: i64 = 60;

/// A code just issued; `code` is shown once and never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCode {
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct PgUnlockTokens {
    pool: PgPool,
}

impl PgUnlockTokens {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// A new code for counter `agent_id`, issued by user `issued_by`, good until `now + TTL_SECS`.
    pub async fn issue_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        agent_id: &str,
        issued_by: &str,
        now: DateTime<Utc>,
    ) -> AppResult<IssuedCode> {
        let code = DeviceSecret::new_pairing_code();
        let hash = DeviceSecret::code_hash(&code).expect("a fresh pairing code is well formed");
        let expires_at = now + Duration::seconds(TTL_SECS);
        sqlx::query(
            "INSERT INTO counter_unlock_tokens (id, agent_id, code_hash, issued_by, issued_at, expires_at) \
             VALUES ($1, $2::uuid, $3, $4::uuid, $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(agent_id)
        .bind(hash)
        .bind(issued_by)
        .bind(now)
        .bind(expires_at)
        .execute(&mut **tx)
        .await?;
        Ok(IssuedCode { code, expires_at })
    }

    /// Uses up `code` for counter `agent_id`. `true` only when it was issued for this counter, is unexpired at `now`
    /// and was not used before; the update is one statement, so two requests cannot both use it.
    pub async fn redeem(&self, agent_id: &str, code: &str, now: DateTime<Utc>) -> AppResult<bool> {
        let Some(hash) = DeviceSecret::code_hash(code) else {
            return Ok(false);
        };
        let done = sqlx::query(
            "UPDATE counter_unlock_tokens SET redeemed_at = $3 \
             WHERE agent_id = $1::uuid AND code_hash = $2 AND redeemed_at IS NULL AND expires_at > $3",
        )
        .bind(agent_id)
        .bind(hash)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}
