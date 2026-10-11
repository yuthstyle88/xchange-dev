//! `ShiftLifecycleHook` — what else happens when a teller's shift opens or closes (#1128, QU-A3).
//!
//! `ShiftService` calls it inside the shift's own transaction, after the shift row is written, so
//! `xc-shift` never names the queue. `xc_queue::application::counter_hook` implements it (a
//! teller's counter follows their shift); the root package installs it.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;

use crate::ids::BranchId;

pub trait ShiftLifecycleHook: Send + Sync {
    /// The shift of `teller_user_id` (`users.id`) has just opened in `branch`, inside `tx`.
    /// An error rolls the open back.
    fn on_opened<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _teller_user_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }

    /// The shift of `teller_user_id` (`users.id`) has just closed in `branch`, inside `tx`.
    /// An error rolls the close back.
    fn on_closed<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _teller_user_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// The hook a `ShiftService` has until one is installed: nothing else happens.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoShiftLifecycleHook;

impl ShiftLifecycleHook for NoShiftLifecycleHook {}
