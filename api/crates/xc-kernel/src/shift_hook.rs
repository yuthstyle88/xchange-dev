//! `ShiftLifecycleHook` — what else happens when a teller's shift opens or closes (#1128, QU-A3),
//! and when the branch day closes with the last one (#4136, design `queue-day-close.md`).
//!
//! `ShiftService` calls it inside the shift's own transaction, after the shift row is written, so
//! `xc-shift` never names the queue. `xc_queue::application::counter_hook` implements it (a
//! teller's counter follows their shift); the root package installs it.

use std::sync::Arc;

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

    /// The branch day of `branch` has just closed: the last OPEN shift's CLOSE count was verified
    /// after cut-off, inside `tx` (#4136). An error rolls the close back.
    fn on_day_closed<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Several hooks as one (#4136): every method calls each hook in order, in the same transaction,
/// and answers the first error without calling the hooks after it.
pub struct ShiftLifecycleHooks {
    hooks: Vec<Arc<dyn ShiftLifecycleHook>>,
}

impl ShiftLifecycleHooks {
    pub fn new(hooks: Vec<Arc<dyn ShiftLifecycleHook>>) -> Self {
        Self { hooks }
    }
}

impl ShiftLifecycleHook for ShiftLifecycleHooks {
    fn on_opened<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            for h in &self.hooks {
                h.on_opened(&mut *tx, branch, teller_user_id, now).await?;
            }
            Ok(())
        })
    }

    fn on_closed<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            for h in &self.hooks {
                h.on_closed(&mut *tx, branch, teller_user_id, now).await?;
            }
            Ok(())
        })
    }

    fn on_day_closed<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            for h in &self.hooks {
                h.on_day_closed(&mut *tx, branch, now).await?;
            }
            Ok(())
        })
    }
}

/// The hook a `ShiftService` has until one is installed: nothing else happens.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoShiftLifecycleHook;

impl ShiftLifecycleHook for NoShiftLifecycleHook {}
