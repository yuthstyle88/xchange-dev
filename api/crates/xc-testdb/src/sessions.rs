//! OR-7.m (#1027): the one test writer of Blueprint sessions (`user_sessions`).
//!
//! Password login goes, so a test signs a user in the way production does after Identity: a
//! `user_sessions` row whose `hash` is the SHA-256 hex of the cookie token, at the user's current
//! `version`. `SessionService::session_check` accepts it while `expires > now` and the user is
//! `ACTIVE` (`xc-auth/src/application/session/session_service.rs:600`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

/// The session cookie's name (`presentation/extractors/actor.rs` `SESSION_COOKIE`).
pub const SESSION_COOKIE: &str = "__Host-xc_session";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Signs `user_id` in: inserts one `user_sessions` row (the user's organisation and current
/// `version`, `branch_id = branch`, `must_change_password = false`, `expires`) and answers the
/// cookie token. Send it as `Cookie: __Host-xc_session=<token>` ([`cookie`]).
///
/// The user must exist; it must be `ACTIVE` for the session to be accepted.
pub async fn seed_session(
    conn: &mut PgConnection,
    user_id: &str,
    branch: Option<&str>,
    expires: DateTime<Utc>,
) -> anyhow::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let token = sha256_hex(&format!("test-session:{user_id}:{nanos}:{n}"));
    sqlx::query(
        "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id, branch_id) \
         SELECT $1, u.id, u.version, $3, u.organization_id, $4 FROM users u WHERE u.id = $2::uuid",
    )
    .bind(sha256_hex(&token))
    .bind(user_id)
    .bind(expires)
    .bind(branch)
    .execute(&mut *conn)
    .await
    .and_then(|done| {
        if done.rows_affected() == 1 {
            Ok(())
        } else {
            Err(sqlx::Error::RowNotFound)
        }
    })?;
    Ok(token)
}

/// The `Cookie` header value for a token from [`seed_session`].
pub fn cookie(token: &str) -> String {
    format!("{SESSION_COOKIE}={token}")
}
