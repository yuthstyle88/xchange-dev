//! The owner's per-counter settings (xc-counter-app.md §4, §5.2, §8.4). Only the Owner saves them
//! (`Permission::BranchSettingsSave`); the store does not check roles, the command does.
//!
//! CA-1d fills `PgCounterStore` in `pg_counter_store.rs`.

use std::future::Future;

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::agent_repository::DeviceAgent;

type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

/// What the owner sets on one counter. Every setting is `None` = not set (§5.2: drawer off, scanner off, kiosk
/// lock on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterSettings {
    /// `device_agents.counter_name`; at most 40 characters, trimmed.
    pub name: String,
    pub cash_drawer: Option<bool>,
    pub scanner: Option<bool>,
    pub kiosk_lock: Option<bool>,
}

pub trait CounterStore: Send + Sync {
    /// The branch's counters that are not revoked, oldest first, then id.
    fn counters(
        &self,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<Vec<DeviceAgent>>> + Send;

    /// Writes `settings` on the active counter `agent_id` of `branch`. The stored counter, or `None` when the
    /// branch has no such active counter.
    fn save_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        agent_id: &str,
        settings: &CounterSettings,
    ) -> impl Future<Output = AppResult<Option<DeviceAgent>>> + Send;
}
