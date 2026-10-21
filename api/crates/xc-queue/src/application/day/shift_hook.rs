//! `QueueDayHook` as the shift's `ShiftLifecycleHook` (#4136, design `queue-day-close.md` §3.4): when the
//! last CLOSE verify closes the branch day, the queue cancels its WAITING and CALLED tickets. Only
//! `on_day_closed` is answered; a teller's shift opening or closing is `CounterService`'s.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::ShiftLifecycleHook;

use super::queue_day_hook::QueueDayHook;

impl ShiftLifecycleHook for QueueDayHook {
    fn on_day_closed<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            QueueDayHook::on_day_closed(self, tx, branch, now)
                .await
                .map(|_| ())
        })
    }
}
