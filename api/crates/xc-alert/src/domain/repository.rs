//! The rate-alert repository port (design §5). Every read and write of `RateAlertService` goes through it. Events
//! are append-only: no update or delete path for them.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_rate::domain::publication::published_rates::PublishedRateLineId;

use super::rate_alert::{RateAlert, RateAlertId};
use super::settings::AlertSettings;
use super::source_policy::SideMode;

/// An alert found by its idempotency key, with the row's `created_at` (the start of its default lifetime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyedAlert {
    pub alert: RateAlert,
    pub created_at: DateTime<Utc>,
}

pub trait RateAlertRepository: Send + Sync {
    /// R13: in the caller's transaction, lock and return the branch's alerts that are due for publication
    /// `version`: enabled, unexpired, undeleted, with `last_publication_version < version` or NULL —
    /// `FOR UPDATE SKIP LOCKED`, so a second transaction at the same time gets none of these rows.
    fn lock_due_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        version: i32,
    ) -> BoxFuture<'a, AppResult<Vec<RateAlert>>>;

    /// The customer's organisation, with the customer row locked `FOR UPDATE`, so two creates of one customer run
    /// one after the other (the limit is counted under this lock). `None` when there is no such customer.
    fn lock_customer_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Uuid>>>;

    /// The organisation's `rate_alert_settings` row, or `None`.
    fn settings_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> BoxFuture<'a, AppResult<Option<AlertSettings>>>;

    /// Inserts or replaces the organisation's one settings row.
    fn save_settings_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        settings: &'a AlertSettings,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// The customer's alert created with `idempotency_key`, with its `created_at`, or `None`.
    fn find_by_key_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<KeyedAlert>>>;

    /// The customer's alerts that count toward the limit at `now`: undeleted and unexpired, paused included.
    fn count_active_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<i64>>;

    /// Writes a new alert with its idempotency key; `created_at` = `updated_at` = `now`.
    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        alert: &'a RateAlert,
        idempotency_key: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// The alert, its row locked `FOR UPDATE`, or `None`.
    fn lock_alert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: RateAlertId,
    ) -> BoxFuture<'a, AppResult<Option<RateAlert>>>;

    /// Writes what pause, resume and delete change: `enabled`, `relation`, `last_publication_version`, `deleted_at`,
    /// `version`, and `updated_at` = `now`.
    fn save_lifecycle_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        alert: &'a RateAlert,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// How the published `side` of `line` was priced (`source_snapshot.buy_mode` / `sell_mode`); AUTO when the
    /// snapshot does not say.
    fn side_mode_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        line: PublishedRateLineId,
        side: Side,
    ) -> BoxFuture<'a, AppResult<SideMode>>;
}
