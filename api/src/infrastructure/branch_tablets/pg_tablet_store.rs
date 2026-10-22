//! `PgBranchTabletStore` (design `kiosk-shared-phone.md` §7.5, §7.6): every statement on
//! `branch_tablets`, and the tablet's changes to `user_sessions`.
//!
//! Ids of `uuid` columns are bound as text with `$n::uuid` and read with `::text`, as
//! `pg_agent_repository.rs` does (sqlx has no `uuid` feature here).
//!
//! Filled by BT-3. `api/tests/branch_tablet_store_test.rs` pins it on a real PostgreSQL.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::tablet_sessions::TabletSessions;

use super::{BranchTablet, TABLET_SESSION_MINUTES, TabletSeen};
use crate::infrastructure::paired_devices::kind::DeviceKind;
use crate::infrastructure::paired_devices::store::{NewPaired, PairedDeviceStore, Tx};

const TABLET_COLUMNS: &str = "id::text AS id, branch_id, name, created_at, created_by, \
     pair_code_expires_at, paired_at, revoked_at, revoked_by, last_seen_at, user_agent, \
     customer_user_id::text AS customer_user_id";

#[derive(sqlx::FromRow)]
struct TabletRow {
    id: String,
    branch_id: String,
    name: String,
    created_at: DateTime<Utc>,
    created_by: String,
    pair_code_expires_at: Option<DateTime<Utc>>,
    paired_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    revoked_by: Option<String>,
    last_seen_at: Option<DateTime<Utc>>,
    user_agent: Option<String>,
    customer_user_id: Option<String>,
}

impl From<TabletRow> for BranchTablet {
    fn from(r: TabletRow) -> Self {
        BranchTablet {
            id: r.id,
            branch_id: r.branch_id,
            name: r.name,
            created_at: r.created_at,
            created_by: r.created_by,
            pair_code_expires_at: r.pair_code_expires_at,
            paired_at: r.paired_at,
            revoked_at: r.revoked_at,
            revoked_by: r.revoked_by,
            last_seen_at: r.last_seen_at,
            user_agent: r.user_agent,
            customer_user_id: r.customer_user_id,
        }
    }
}

#[derive(Clone)]
pub struct PgBranchTabletStore {
    pool: PgPool,
}

impl PgBranchTabletStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// One transaction's work at `otp/verify` on a paired tablet (T-D7, T14, T23), for the live
    /// (not revoked, paired) tablet `tablet_id`:
    /// - the session bound before, if any and if it is not `session_hash`, is deleted from
    ///   `user_sessions`;
    /// - `customer_session_hash = session_hash`, `customer_user_id = user_id`;
    /// - the new session's `user_sessions.expires` becomes at most `now + TABLET_SESSION_MINUTES`.
    ///
    /// `false` (and nothing written) when the tablet is revoked, unpaired or unknown.
    pub async fn bind_session_in(
        &self,
        tx: Tx<'_, '_>,
        tablet_id: &str,
        session_hash: &str,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let _ = &self.pool;
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT customer_session_hash FROM branch_tablets \
             WHERE id = $1::uuid AND paired_at IS NOT NULL AND revoked_at IS NULL \
             FOR UPDATE",
        )
        .bind(tablet_id)
        .fetch_optional(&mut **tx)
        .await?;
        let Some((prev_hash,)) = row else {
            return Ok(false);
        };
        if let Some(prev) = prev_hash.filter(|h| h != session_hash) {
            sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                .bind(prev)
                .execute(&mut **tx)
                .await?;
        }
        sqlx::query(
            "UPDATE branch_tablets \
             SET customer_session_hash = NULL, customer_user_id = NULL \
             WHERE customer_session_hash = $1 AND id <> $2::uuid",
        )
        .bind(session_hash)
        .bind(tablet_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE branch_tablets \
             SET customer_session_hash = $2, customer_user_id = $3::uuid \
             WHERE id = $1::uuid",
        )
        .bind(tablet_id)
        .bind(session_hash)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
        let cap = now + Duration::minutes(TABLET_SESSION_MINUTES);
        sqlx::query("UPDATE user_sessions SET expires = LEAST(expires, $2) WHERE hash = $1")
            .bind(session_hash)
            .bind(cap)
            .execute(&mut **tx)
            .await?;
        Ok(true)
    }

    /// The scan's and the cancel's end (T-D7, T15, T16, T18), inside `tx`: as
    /// `TabletSessions::end_for_member_in` documents. Only tablets of `branch` are touched.
    pub async fn end_for_member_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        customer_id: &str,
    ) -> AppResult<u64> {
        let hashes: Vec<(String,)> = sqlx::query_as(
            "WITH cleared AS ( \
                 UPDATE branch_tablets b \
                 SET customer_session_hash = NULL, customer_user_id = NULL \
                 FROM ( \
                     SELECT id, customer_session_hash FROM branch_tablets \
                     WHERE branch_id = $1 AND customer_user_id = ( \
                         SELECT user_id FROM customers WHERE id = $2 AND user_id IS NOT NULL \
                     ) \
                 ) old \
                 WHERE b.id = old.id \
                 RETURNING old.customer_session_hash \
             ) \
             SELECT customer_session_hash FROM cleared WHERE customer_session_hash IS NOT NULL",
        )
        .bind(branch.as_str())
        .bind(customer_id)
        .fetch_all(&mut **tx)
        .await?;
        let count = hashes.len() as u64;
        for (hash,) in hashes {
            sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                .bind(hash)
                .execute(&mut **tx)
                .await?;
        }
        Ok(count)
    }

    /// Whether `session_hash` is the customer session bound to some tablet, revoked or not
    /// (T23: such a session is never refreshed).
    pub async fn is_bound_session(&self, session_hash: &str) -> AppResult<bool> {
        let found: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM branch_tablets WHERE customer_session_hash = $1)",
        )
        .bind(session_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(found)
    }

    /// One statement: a live tablet's `last_seen_at = now` and `user_agent` (§7.7 `me`).
    /// `false` when the tablet is revoked or unknown.
    pub async fn touch(
        &self,
        tablet_id: &str,
        seen: &TabletSeen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "UPDATE branch_tablets SET last_seen_at = $2, user_agent = $3 \
             WHERE id = $1::uuid AND revoked_at IS NULL",
        )
        .bind(tablet_id)
        .bind(now)
        .bind(seen.user_agent)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}

impl PairedDeviceStore for PgBranchTabletStore {
    const KIND: DeviceKind = DeviceKind::BranchTablet;
    type Device = BranchTablet;
    type Seen<'a> = TabletSeen<'a>;

    async fn insert_pending_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        device: &NewPaired<'_>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "INSERT INTO branch_tablets \
             (id, branch_id, name, created_at, created_by, pair_code_hash, pair_code_expires_at) \
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7)",
        )
        .bind(device.id)
        .bind(branch.as_str())
        .bind(device.name)
        .bind(device.at)
        .bind(device.created_by)
        .bind(device.pair_code_hash)
        .bind(device.pair_code_expires_at)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn set_code_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        pair_code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "UPDATE branch_tablets SET pair_code_hash = $3, pair_code_expires_at = $4 \
             WHERE branch_id = $1 AND id = $2::uuid AND revoked_at IS NULL",
        )
        .bind(branch.as_str())
        .bind(id)
        .bind(pair_code_hash)
        .bind(expires_at)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn pair(
        &self,
        pair_code_hash: &str,
        token_hash: &str,
        seen: &Self::Seen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<BranchTablet>> {
        let row: Option<TabletRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE branch_tablets SET token_hash = $2, paired_at = $3, last_seen_at = $3, \
             pair_code_hash = NULL, pair_code_expires_at = NULL, user_agent = $4 \
             WHERE pair_code_hash = $1 AND revoked_at IS NULL AND pair_code_expires_at > $3 \
             RETURNING {TABLET_COLUMNS}"
        )))
        .bind(pair_code_hash)
        .bind(token_hash)
        .bind(now)
        .bind(seen.user_agent)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(BranchTablet::from))
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> AppResult<Option<BranchTablet>> {
        let row: Option<TabletRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {TABLET_COLUMNS} FROM branch_tablets WHERE token_hash = $1"
        )))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(BranchTablet::from))
    }

    async fn find(&self, id: &str) -> AppResult<Option<BranchTablet>> {
        let row: Option<TabletRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {TABLET_COLUMNS} FROM branch_tablets WHERE id = $1::uuid"
        )))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(BranchTablet::from))
    }

    async fn revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "UPDATE branch_tablets SET revoked_at = $4, revoked_by = $3 \
             WHERE branch_id = $1 AND id = $2::uuid AND revoked_at IS NULL",
        )
        .bind(branch.as_str())
        .bind(id)
        .bind(actor)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Deletes the tablet's bound customer session, if any, and clears its `customer_*` columns.
    async fn after_revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let _ = (actor, now);
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "UPDATE branch_tablets \
             SET customer_session_hash = NULL, customer_user_id = NULL \
             WHERE branch_id = $1 AND id = $2::uuid \
             RETURNING (SELECT customer_session_hash FROM branch_tablets WHERE id = $2::uuid)",
        )
        .bind(branch.as_str())
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some((Some(hash),)) = row {
            sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                .bind(hash)
                .execute(&mut **tx)
                .await?;
        }
        Ok(())
    }

    async fn list(&self) -> AppResult<Vec<BranchTablet>> {
        let rows: Vec<TabletRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {TABLET_COLUMNS} FROM branch_tablets ORDER BY branch_id, created_at, id"
        )))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(BranchTablet::from).collect())
    }
}

impl TabletSessions for PgBranchTabletStore {
    fn end_for_member_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<u64>> + Send + 'a>> {
        Box::pin(Self::end_for_member_in(self, tx, branch, customer_id))
    }
}
