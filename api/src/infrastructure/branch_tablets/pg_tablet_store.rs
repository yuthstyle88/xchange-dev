//! `PgBranchTabletStore` (design `kiosk-shared-phone.md` §7.5, §7.6): every statement on
//! `branch_tablets`, and the tablet's changes to `user_sessions`.
//!
//! Ids of `uuid` columns are bound as text with `$n::uuid` and read with `::text`, as
//! `pg_agent_repository.rs` does (sqlx has no `uuid` feature here).
//!
//! Filled by BT-3. `api/tests/branch_tablet_store_test.rs` pins it on a real PostgreSQL.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::tablet_sessions::TabletSessions;

use super::{BranchTablet, TabletSeen};
use crate::infrastructure::paired_devices::kind::DeviceKind;
use crate::infrastructure::paired_devices::store::{NewPaired, PairedDeviceStore, Tx};

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
        let _ = (&self.pool, tx, tablet_id, session_hash, user_id, now);
        todo!("BT-3: PgBranchTabletStore::bind_session_in")
    }

    /// The scan's and the cancel's end (T-D7, T15, T16, T18), inside `tx`: as
    /// `TabletSessions::end_for_member_in` documents. Only tablets of `branch` are touched.
    pub async fn end_for_member_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        customer_id: &str,
    ) -> AppResult<u64> {
        let _ = (tx, branch, customer_id);
        todo!("BT-3: PgBranchTabletStore::end_for_member_in")
    }

    /// Whether `session_hash` is the customer session bound to some tablet, revoked or not
    /// (T23: such a session is never refreshed).
    pub async fn is_bound_session(&self, session_hash: &str) -> AppResult<bool> {
        let _ = session_hash;
        todo!("BT-3: PgBranchTabletStore::is_bound_session")
    }

    /// One statement: a live tablet's `last_seen_at = now` and `user_agent` (§7.7 `me`).
    /// `false` when the tablet is revoked or unknown.
    pub async fn touch(
        &self,
        tablet_id: &str,
        seen: &TabletSeen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let _ = (tablet_id, seen, now);
        todo!("BT-3: PgBranchTabletStore::touch")
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
        let _ = (tx, branch, device);
        todo!("BT-3: PgBranchTabletStore::insert_pending_in")
    }

    async fn set_code_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        pair_code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let _ = (tx, branch, id, pair_code_hash, expires_at);
        todo!("BT-3: PgBranchTabletStore::set_code_in")
    }

    async fn pair(
        &self,
        pair_code_hash: &str,
        token_hash: &str,
        seen: &Self::Seen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<BranchTablet>> {
        let _ = (pair_code_hash, token_hash, seen, now);
        todo!("BT-3: PgBranchTabletStore::pair")
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> AppResult<Option<BranchTablet>> {
        let _ = token_hash;
        todo!("BT-3: PgBranchTabletStore::find_by_token_hash")
    }

    async fn find(&self, id: &str) -> AppResult<Option<BranchTablet>> {
        let _ = id;
        todo!("BT-3: PgBranchTabletStore::find")
    }

    async fn revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let _ = (tx, branch, id, actor, now);
        todo!("BT-3: PgBranchTabletStore::revoke_in")
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
        let _ = (tx, branch, id, actor, now);
        todo!("BT-3: PgBranchTabletStore::after_revoke_in")
    }

    async fn list(&self) -> AppResult<Vec<BranchTablet>> {
        todo!("BT-3: PgBranchTabletStore::list")
    }
}

impl TabletSessions for PgBranchTabletStore {
    fn end_for_member_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<u64>> + Send + 'a>> {
        let _ = (tx, branch, customer_id);
        todo!("BT-3: TabletSessions for PgBranchTabletStore (delegates to the inherent method)")
    }
}
