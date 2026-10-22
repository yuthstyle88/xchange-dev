//! `PairedDeviceStore` (design `kiosk-shared-phone.md` §7.5): every statement the pairing core
//! needs on one kind's table. The drawer agent's `PgAgentRepository` (BT-2) and the branch
//! tablet's `PgBranchTabletStore` (BT-3) implement it; `DevicePairing<S>` (`pairing.rs`) is the
//! only caller.
//!
//! Each statement keeps the rule the drawer's statement has today
//! (`pg_agent_repository.rs`), so one rule holds for both kinds.

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::kind::DeviceKind;

/// A transaction of one branch, as `BranchTx::run` hands it.
pub type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

/// What the core reads of a stored device. No hash is ever handed out.
pub trait PairedDevice {
    /// A UUID, lower case with dashes.
    fn id(&self) -> &str;
    fn branch_id(&self) -> &str;
    fn name(&self) -> &str;
    /// `revoked_at` is set.
    fn is_revoked(&self) -> bool;
}

/// A device about to be stored, unpaired: the staff's `add`.
#[derive(Debug, Clone)]
pub struct NewPaired<'a> {
    /// A UUID, lower case with dashes.
    pub id: &'a str,
    /// Trimmed, 1 to 60 UTF-16 units (`agent_admin.rs:151-155`).
    pub name: &'a str,
    pub created_by: &'a str,
    /// `DeviceSecret::code_hash` of the code shown once.
    pub pair_code_hash: &'a str,
    /// `at` + `CODE_VALID_MINUTES`.
    pub pair_code_expires_at: DateTime<Utc>,
    pub at: DateTime<Utc>,
}

pub trait PairedDeviceStore: Send + Sync + 'static {
    /// The kind this store holds; it decides the token prefix, the attempt keys and
    /// `per_branch`.
    const KIND: DeviceKind;

    /// One stored device.
    type Device: PairedDevice + Clone + Send + Sync + 'static;

    /// What the device says about itself when it pairs (the drawer's `AgentSeen`, the tablet's
    /// user agent).
    type Seen<'a>: Sync;

    /// Stores a new, unpaired device of the branch. With `KIND.per_branch() == One` nothing is
    /// written while the branch has a device that is not revoked (a pending one counts).
    /// `true` when it was stored.
    fn insert_pending_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        device: &NewPaired<'_>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Gives a device of the branch that is not revoked a new pairing code (re-pair). The token
    /// it holds stays valid until the new code is used. `true` when such a device exists.
    fn set_code_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        pair_code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// One statement: the device that is not revoked, whose code hashes to `pair_code_hash` and
    /// has not expired at `now` (`pair_code_expires_at > now`), takes `token_hash` (voiding the
    /// token it held), `paired_at = now`, `last_seen_at = now` and what it says about itself;
    /// its code is used up (both code columns `NULL`). `None` when no such device: unknown,
    /// expired and used codes are one answer.
    fn pair(
        &self,
        pair_code_hash: &str,
        token_hash: &str,
        seen: &Self::Seen<'_>,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<Self::Device>>> + Send;

    /// The device whose token hashes to `token_hash`, revoked or not.
    fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = AppResult<Option<Self::Device>>> + Send;

    /// One device by id, revoked or not. `id` is a UUID already parsed by the caller.
    fn find(&self, id: &str) -> impl Future<Output = AppResult<Option<Self::Device>>> + Send;

    /// Revokes a device of the branch that is not revoked: `revoked_at = now`,
    /// `revoked_by = actor`. The row keeps its hashes for the audit trail. `true` when such a
    /// device existed.
    fn revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// What else a revoke does, in the same transaction, called only after `revoke_in` answered
    /// `true`: the drawer fails the branch's `QUEUED` drawer jobs (`AGENT_REVOKED`); the tablet
    /// deletes its bound customer session.
    fn after_revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Every device of this kind, by branch id, then oldest first, then id.
    fn list(&self) -> impl Future<Output = AppResult<Vec<Self::Device>>> + Send;
}
