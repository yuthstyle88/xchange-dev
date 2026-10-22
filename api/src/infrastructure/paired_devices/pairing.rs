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

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use uuid::Uuid;
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use super::secret::DeviceSecret;
use super::store::{NewPaired, PairedDevice, PairedDeviceStore};
use super::{
    ATTEMPT_WINDOW_MINUTES, CODE_VALID_MINUTES, PAIR_ATTEMPTS_PER_WINDOW, TOKEN_FAILURES_PER_WINDOW,
};

fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let millis = now.timestamp_millis();
    let window_millis = millis.div_euclid(ATTEMPT_WINDOW_MINUTES * 60 * 1000)
        * (ATTEMPT_WINDOW_MINUTES * 60 * 1000);
    DateTime::from_timestamp_millis(window_millis).unwrap_or(now)
}

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
        self.branches.require(branch).await?;

        let trimmed_name = name.trim();
        let units = trimmed_name.encode_utf16().count();
        if units == 0 || units > 60 {
            return Err(PairingError::NameRequired);
        }

        let id = Uuid::new_v4().to_string();
        let code = DeviceSecret::new_pairing_code();
        let code_hash = DeviceSecret::code_hash(&code)
            .ok_or_else(|| AppError::Internal("failed to hash pairing code".to_owned()))?;
        let expires_at = now + Duration::minutes(CODE_VALID_MINUTES);

        let store = self.store.clone();
        let b = branch.clone();
        let id_clone = id.clone();
        let name_string = trimmed_name.to_owned();
        let actor_string = actor.to_owned();

        let stored = self
            .branch_tx
            .run(branch, move |tx| {
                let store = store.clone();
                let b = b.clone();
                let id_clone = id_clone.clone();
                let name_string = name_string.clone();
                let actor_string = actor_string.clone();
                let code_hash = code_hash.clone();
                Box::pin(async move {
                    let new_paired = NewPaired {
                        id: &id_clone,
                        name: &name_string,
                        created_by: &actor_string,
                        pair_code_hash: &code_hash,
                        pair_code_expires_at: expires_at,
                        at: now,
                    };
                    store.insert_pending_in(tx, &b, &new_paired).await
                })
            })
            .await?;

        if !stored {
            return Err(PairingError::BranchHasOne);
        }

        Ok(PairingCode {
            device_id: id,
            code,
            code_expires_at: expires_at,
        })
    }

    /// A new code for device `id` (re-pair), valid `CODE_VALID_MINUTES` from `now`.
    pub async fn renew(
        &self,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<PairingCode, PairingError> {
        let _ = actor;
        let Ok(uuid) = Uuid::parse_str(id) else {
            return Err(PairingError::NoSuchDevice);
        };
        let id_str = uuid.to_string();
        let Some(device) = self.store.find(&id_str).await? else {
            return Err(PairingError::NoSuchDevice);
        };
        if device.is_revoked() {
            return Err(PairingError::NoSuchDevice);
        }

        let branch = BranchId::new(device.branch_id());
        let code = DeviceSecret::new_pairing_code();
        let code_hash = DeviceSecret::code_hash(&code)
            .ok_or_else(|| AppError::Internal("failed to hash pairing code".to_owned()))?;
        let expires_at = now + Duration::minutes(CODE_VALID_MINUTES);

        let store = self.store.clone();
        let b = branch.clone();
        let id_clone = id_str.clone();
        let hash_clone = code_hash.clone();

        let updated = self
            .branch_tx
            .run(&branch, move |tx| {
                let store = store.clone();
                let b = b.clone();
                let id_clone = id_clone.clone();
                let hash_clone = hash_clone.clone();
                Box::pin(async move {
                    store
                        .set_code_in(tx, &b, &id_clone, &hash_clone, expires_at)
                        .await
                })
            })
            .await?;

        if !updated {
            return Err(PairingError::NoSuchDevice);
        }

        Ok(PairingCode {
            device_id: id_str,
            code,
            code_expires_at: expires_at,
        })
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
        let key = DeviceSecret::hash(&S::KIND.pair_key(address));
        let win = window_start(now);
        let count = self.attempts.increment(&key, win).await?;
        if count > PAIR_ATTEMPTS_PER_WINDOW {
            return Ok(PairOutcome::RateLimited);
        }

        let Some(code_hash) = DeviceSecret::code_hash(code) else {
            return Ok(PairOutcome::InvalidCode);
        };

        let token = DeviceSecret::new_token(S::KIND);
        let token_hash = DeviceSecret::hash(&token);

        let Some(device) = self.store.pair(&code_hash, &token_hash, seen, now).await? else {
            return Ok(PairOutcome::InvalidCode);
        };

        Ok(PairOutcome::Success { device, token })
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
        let Some(raw_token) = token else {
            return self.record_failed_auth(address, now).await;
        };

        let trimmed = raw_token.trim();
        if !trimmed.starts_with(S::KIND.token_prefix()) {
            return self.record_failed_auth(address, now).await;
        }

        let token_hash = DeviceSecret::hash(trimmed);
        let Some(device) = self.store.find_by_token_hash(&token_hash).await? else {
            return self.record_failed_auth(address, now).await;
        };

        if device.is_revoked() {
            return self.record_failed_auth(address, now).await;
        }

        Ok(AuthOutcome::Ok(Box::new(device)))
    }

    async fn record_failed_auth(
        &self,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<AuthOutcome<S::Device>> {
        let key = DeviceSecret::hash(&S::KIND.token_key(address));
        let win = window_start(now);
        let count = self.attempts.increment(&key, win).await?;
        if count > TOKEN_FAILURES_PER_WINDOW {
            Ok(AuthOutcome::RateLimited)
        } else {
            Ok(AuthOutcome::InvalidToken)
        }
    }

    /// Revokes device `id`, then `after_revoke_in`, in one transaction of its branch.
    pub async fn revoke(
        &self,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<(), PairingError> {
        let Ok(uuid) = Uuid::parse_str(id) else {
            return Err(PairingError::NoSuchDevice);
        };
        let id_str = uuid.to_string();
        let Some(device) = self.store.find(&id_str).await? else {
            return Err(PairingError::NoSuchDevice);
        };
        if device.is_revoked() {
            return Err(PairingError::NoSuchDevice);
        }

        let branch = BranchId::new(device.branch_id());
        let store = self.store.clone();
        let b = branch.clone();
        let id_clone = id_str.clone();
        let actor_str = actor.to_owned();

        let revoked = self
            .branch_tx
            .run(&branch, move |tx| {
                let store = store.clone();
                let b = b.clone();
                let id_clone = id_clone.clone();
                let actor_str = actor_str.clone();
                Box::pin(async move {
                    if !store.revoke_in(tx, &b, &id_clone, &actor_str, now).await? {
                        return Ok(false);
                    }
                    store
                        .after_revoke_in(tx, &b, &id_clone, &actor_str, now)
                        .await?;
                    Ok(true)
                })
            })
            .await?;

        if !revoked {
            return Err(PairingError::NoSuchDevice);
        }

        Ok(())
    }

    /// The branch of device `id`, or `None` when `id` is not a UUID or names no device (revoked
    /// ones included), so the route can authorise the branch before anything else is read.
    pub async fn branch_of(&self, id: &str) -> AppResult<Option<BranchId>> {
        let Ok(uuid) = Uuid::parse_str(id) else {
            return Ok(None);
        };
        let id_str = uuid.to_string();
        let Some(device) = self.store.find(&id_str).await? else {
            return Ok(None);
        };
        Ok(Some(BranchId::new(device.branch_id())))
    }
}
