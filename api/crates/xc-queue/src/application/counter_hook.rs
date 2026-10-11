//! `CounterService` as the shift's `ShiftLifecycleHook` (#1128, QU-A3): opening a teller's shift
//! staffs their counter (`on_shift_opened`), closing it releases the counter (`on_shift_closed`).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::ShiftLifecycleHook;

use crate::application::counter::counter_service::CounterService;

impl ShiftLifecycleHook for CounterService {
    fn on_opened<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            self.on_shift_opened(tx, branch, teller_user_id, now)
                .await
                .map(|_| ())
        })
    }

    fn on_closed<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move { self.on_shift_closed(tx, branch, teller_user_id, now).await })
    }
}
