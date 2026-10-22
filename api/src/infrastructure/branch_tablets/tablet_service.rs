//! `BranchTabletService` (design `kiosk-shared-phone.md` §7.3 T-D6, T-D7; §7.5; §7.7): the
//! tablet's side of the server above the store, on the pairing core
//! `DevicePairing<PgBranchTabletStore>`.
//!
//! Filled by BT-3. `api/tests/branch_tablet_store_test.rs` pins it on a real PostgreSQL; the
//! routes (BT-4) and the sign-in binding in `auth.rs` (BT-5) call it.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;

use super::pg_tablet_store::PgBranchTabletStore;
use super::{ListedTablet, TabletMe, TabletSeen};
use crate::infrastructure::paired_devices::pairing::DevicePairing;

#[derive(Clone)]
pub struct BranchTabletService {
    store: Arc<PgBranchTabletStore>,
    pairing: DevicePairing<PgBranchTabletStore>,
}

impl BranchTabletService {
    /// The service on `branch_tx`'s pool: one store, the pairing core over it, and the
    /// `auth_attempts` counter.
    pub fn new(branch_tx: BranchTx) -> Self {
        let pool = branch_tx.pool().clone();
        let store = Arc::new(PgBranchTabletStore::new(pool.clone()));
        let pairing = DevicePairing::new(
            store.clone(),
            branch_tx,
            PgAttemptCounter::new(pool.clone()),
        );
        Self { store, pairing }
    }

    /// The pairing core: `issue` (`add`), `renew` (`newCode`), `pair`, `revoke`, `branch_of`.
    pub fn pairing(&self) -> &DevicePairing<PgBranchTabletStore> {
        &self.pairing
    }

    /// The store: `bind_session_in`, `end_for_member_in` (the `TabletSessions` port).
    pub fn store(&self) -> &PgBranchTabletStore {
        &self.store
    }

    /// Every tablet with its state: `REVOKED` if revoked, otherwise `PENDING` if never paired,
    /// otherwise `PAIRED`. By branch id, then oldest first, then id.
    pub async fn list(&self, now: DateTime<Utc>) -> AppResult<Vec<ListedTablet>> {
        let _ = now;
        todo!("BT-3: BranchTabletService::list")
    }

    /// `GET /api/branch-tablet/me` (T-D6, §7.7): the tablet holding `token` (the cookie's value),
    /// through `DevicePairing::authenticate` with `address`. A live one is touched
    /// (`last_seen_at`, `user_agent` from `seen`) and answered `Paired`. No token ⇒
    /// `Unpaired { clear_cookie: false }`; any other failure, the rate limit included ⇒
    /// `Unpaired { clear_cookie: true }`.
    pub async fn me(
        &self,
        token: Option<&str>,
        seen: &TabletSeen<'_>,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<TabletMe> {
        let _ = (token, seen, address, now);
        todo!("BT-3: BranchTabletService::me")
    }

    /// The sign-in binding (T-D7, T14, T23): when `token` (the tablet cookie's value) is a live
    /// tablet's, binds the new session `session_hash` (`user_sessions.hash`) of `user_id` to it
    /// with `PgBranchTabletStore::bind_session_in`, in one transaction of the tablet's branch.
    /// `false` (nothing written) for any other token. Counts no attempt: `otp/verify` has its own
    /// limits.
    pub async fn bind_on_sign_in(
        &self,
        token: &str,
        session_hash: &str,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let _ = (token, session_hash, user_id, now);
        todo!("BT-3: BranchTabletService::bind_on_sign_in")
    }

    /// Whether `session_hash` is bound to a tablet (T23: `POST /api/auth/refresh` refuses it).
    pub async fn is_bound_session(&self, session_hash: &str) -> AppResult<bool> {
        let _ = session_hash;
        todo!("BT-3: BranchTabletService::is_bound_session")
    }
}
