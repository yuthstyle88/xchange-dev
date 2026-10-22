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
use xc_kernel::ids::BranchId;

use super::pg_tablet_store::PgBranchTabletStore;
use super::{ListedTablet, TabletMe, TabletSeen, TabletState};
use crate::infrastructure::paired_devices::kind::DeviceKind;
use crate::infrastructure::paired_devices::pairing::{AuthOutcome, DevicePairing};
use crate::infrastructure::paired_devices::secret::DeviceSecret;
use crate::infrastructure::paired_devices::store::PairedDeviceStore;

#[derive(Clone)]
pub struct BranchTabletService {
    store: Arc<PgBranchTabletStore>,
    pairing: DevicePairing<PgBranchTabletStore>,
    branch_tx: BranchTx,
}

impl BranchTabletService {
    /// The service on `branch_tx`'s pool: one store, the pairing core over it, and the
    /// `auth_attempts` counter.
    pub fn new(branch_tx: BranchTx) -> Self {
        let pool = branch_tx.pool().clone();
        let store = Arc::new(PgBranchTabletStore::new(pool.clone()));
        let pairing = DevicePairing::new(
            store.clone(),
            branch_tx.clone(),
            PgAttemptCounter::new(pool),
        );
        Self {
            store,
            pairing,
            branch_tx,
        }
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
        let all = self.store.list().await?;
        Ok(all
            .into_iter()
            .map(|t| {
                let state = if t.revoked_at.is_some() {
                    TabletState::Revoked
                } else if t.paired_at.is_none() {
                    TabletState::Pending
                } else {
                    TabletState::Paired
                };
                ListedTablet {
                    tablet_id: t.id,
                    branch_id: t.branch_id,
                    name: t.name,
                    state,
                    last_seen_at: t.last_seen_at,
                    created_at: t.created_at,
                    paired_at: t.paired_at,
                    revoked_at: t.revoked_at,
                }
            })
            .collect())
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
        let Some(token) = token else {
            return Ok(TabletMe::Unpaired {
                clear_cookie: false,
            });
        };
        match self.pairing.authenticate(Some(token), address, now).await? {
            AuthOutcome::Ok(tablet) => {
                if !self.store.touch(&tablet.id, seen, now).await? {
                    return Ok(TabletMe::Unpaired { clear_cookie: true });
                }
                Ok(TabletMe::Paired {
                    tablet_id: tablet.id,
                    branch_code: tablet.branch_id,
                    name: tablet.name,
                })
            }
            AuthOutcome::RateLimited | AuthOutcome::InvalidToken => {
                Ok(TabletMe::Unpaired { clear_cookie: true })
            }
        }
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
        let trimmed = token.trim();
        if !trimmed.starts_with(DeviceKind::BranchTablet.token_prefix()) {
            return Ok(false);
        }
        let token_hash = DeviceSecret::hash(trimmed);
        let Some(tablet) = self.store.find_by_token_hash(&token_hash).await? else {
            return Ok(false);
        };
        if tablet.revoked_at.is_some() || tablet.paired_at.is_none() {
            return Ok(false);
        }
        let branch = BranchId::new(&tablet.branch_id);
        let store = self.store.clone();
        let tablet_id = tablet.id;
        let session_hash = session_hash.to_owned();
        let user_id = user_id.to_owned();
        self.branch_tx
            .run(&branch, move |tx| {
                let store = store.clone();
                let tablet_id = tablet_id.clone();
                let session_hash = session_hash.clone();
                let user_id = user_id.clone();
                Box::pin(async move {
                    store
                        .bind_session_in(tx, &tablet_id, &session_hash, &user_id, now)
                        .await
                })
            })
            .await
    }

    /// Whether `session_hash` is bound to a tablet (T23: `POST /api/auth/refresh` refuses it).
    pub async fn is_bound_session(&self, session_hash: &str) -> AppResult<bool> {
        self.store.is_bound_session(session_hash).await
    }
}
