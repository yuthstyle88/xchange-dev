//! `DevicePairing<S>` (design `kiosk-shared-phone.md` §7.3 T-D2, §7.5): issue a code, re-pair,
//! pair, authenticate a token and revoke, once for every kind of paired device. The rules are
//! today's drawer agent's (`agent_admin.rs:140-319`, `agent_service.rs:82-166`), unchanged:
//! - a code is 10 Crockford characters, valid `CODE_VALID_MINUTES`, single use, stored as SHA-256;
//! - unknown, expired and used codes are one answer (`InvalidCode`);
//! - `PAIR_ATTEMPTS_PER_WINDOW` pair attempts per address and 15-minute window, counted in
//!   `auth_attempts` under `DeviceSecret::hash(S::KIND.pair_key(address))`, every attempt counted;
//! - `TOKEN_FAILURES_PER_WINDOW` failed token checks per address and window, under
//!   `DeviceSecret::hash(S::KIND.token_key(address))`, only failures counted;
//! - a token without `S::KIND.token_prefix()` fails before any lookup;
//! - a new code re-pairs the device: the old token works until the new code is used, then never.
//!
//! Filled by BT-1. `api/tests/paired_device_core_test.rs` pins it for both kinds through one
//! table, with a fake store.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use super::store::PairedDeviceStore;

/// A pairing code answered by `issue` or `renew`. Shown once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub device_id: String,
    /// `XXXXX-XXXXX`.
    pub code: String,
    pub code_expires_at: DateTime<Utc>,
}

/// Outcome of a pair attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairOutcome<D> {
    /// The device as stored after pairing, and its new token: answered once, never stored.
    Success {
        device: D,
        token: String,
    },
    RateLimited,
    InvalidCode,
}

/// Outcome of a token check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome<D> {
    Ok(Box<D>),
    RateLimited,
    InvalidToken,
}

/// Why `issue`, `renew` or `revoke` refused. Each kind words it for its own screen (the drawer:
/// `agent_admin.rs:16-18`).
#[derive(Debug)]
pub enum PairingError {
    /// The trimmed name is empty or longer than 60 UTF-16 units.
    NameRequired,
    /// `PerBranch::One` and the branch already has a device that is not revoked.
    BranchHasOne,
    /// The id is not a UUID, names no device, or names a revoked one.
    NoSuchDevice,
    /// The branch does not exist, or the database failed.
    App(AppError),
}

impl From<AppError> for PairingError {
    fn from(e: AppError) -> Self {
        Self::App(e)
    }
}

/// The pairing core for the kind `S` stores.
pub struct DevicePairing<S> {
    store: Arc<S>,
    branch_tx: BranchTx,
    attempts: PgAttemptCounter,
    branches: BranchService,
}

impl<S> Clone for DevicePairing<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            branch_tx: self.branch_tx.clone(),
            attempts: self.attempts.clone(),
            branches: self.branches.clone(),
        }
    }
}

impl<S: PairedDeviceStore> DevicePairing<S> {
    /// The core over `store`, writing in `branch_tx`'s branch transactions and counting attempts
    /// in `attempts`. The branch check is `BranchService::require` on the same pool.
    pub fn new(store: Arc<S>, branch_tx: BranchTx, attempts: PgAttemptCounter) -> Self {
        let branches = BranchService::new(branch_tx.clone());
        Self {
            store,
            branch_tx,
            attempts,
            branches,
        }
    }

    /// The store this core writes through.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Adds an unpaired device named `name` (trimmed) to `branch` with a new code valid
    /// `CODE_VALID_MINUTES` from `now`. The branch must exist. Only the code's hash is stored.
    pub async fn issue(
        &self,
        branch: &BranchId,
        name: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<PairingCode, PairingError> {
        let _ = (
            &self.store,
            &self.branch_tx,
            &self.attempts,
            &self.branches,
            branch,
            name,
            actor,
            now,
        );
        todo!("BT-1: DevicePairing::issue")
    }

    /// A new code for device `id` (re-pair), valid `CODE_VALID_MINUTES` from `now`.
    pub async fn renew(
        &self,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<PairingCode, PairingError> {
        let _ = (id, actor, now);
        todo!("BT-1: DevicePairing::renew")
    }

    /// Pairs the device whose live code is `code` (any case, with or without the dash) from
    /// `address`. Counts the attempt first; past the limit nothing else is done.
    pub async fn pair(
        &self,
        code: &str,
        seen: &S::Seen<'_>,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<PairOutcome<S::Device>> {
        let _ = (code, seen, address, now);
        todo!("BT-1: DevicePairing::pair")
    }

    /// The device holding `token` (trimmed), if it is not revoked. A missing token, one without
    /// the kind's prefix, an unknown one and a revoked device's are each one failure from
    /// `address`.
    pub async fn authenticate(
        &self,
        token: Option<&str>,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<AuthOutcome<S::Device>> {
        let _ = (token, address, now);
        todo!("BT-1: DevicePairing::authenticate")
    }

    /// Revokes device `id`, then `after_revoke_in`, in one transaction of its branch.
    pub async fn revoke(
        &self,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<(), PairingError> {
        let _ = (id, actor, now);
        todo!("BT-1: DevicePairing::revoke")
    }

    /// The branch of device `id`, or `None` when `id` is not a UUID or names no device (revoked
    /// ones included), so the route can authorise the branch before anything else is read.
    pub async fn branch_of(&self, id: &str) -> AppResult<Option<BranchId>> {
        let _ = id;
        todo!("BT-1: DevicePairing::branch_of")
    }
}
