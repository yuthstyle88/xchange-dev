//! `KioskSettingsRepository` (design `kiosk-shared-phone.md` §4.2 D14): the per-branch queue-machine
//! settings. The root package implements it (`PgKioskSettingsRepository`).

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::kiosk::entities::kiosk_settings::KioskSettings;

type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

pub trait KioskSettingsRepository: Send + Sync {
    /// The stored row of `branch`; `None` when there is none (the caller answers
    /// `KioskSettings::defaults`).
    fn get(
        &self,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<Option<KioskSettings>>> + Send;

    /// Inserts or replaces the branch's row inside `tx`: `result_seconds` (already validated),
    /// `updated_by = by`, `updated_at = now`, `version` + 1 (1 on the first save). Answers the row
    /// as stored.
    fn upsert_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        result_seconds: i32,
        by: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<KioskSettings>> + Send;
}
