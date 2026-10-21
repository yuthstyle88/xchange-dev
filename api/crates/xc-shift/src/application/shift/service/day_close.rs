//! #4136 (design `queue-day-close.md` §3.2): after the CLOSE verify closes a shift, the branch day closes
//! with it when the stored phase is CUT_OFF and no shift is OPEN any more. Only the CLOSE verify asks;
//! a handover closes the giver through `close_on_verified` and never comes here.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::ShiftService;
use crate::domain::shift::repositories::shift_repository::ShiftRepository;

impl<R: ShiftRepository> ShiftService<R> {
    /// Runs in the caller's transaction, after the shift's `UPDATE`. Locks the branch's `ops_state` row
    /// (`FOR UPDATE`) so two concurrent closes fire once, counts the OPEN shifts, and when
    /// `BranchDay::day_closes_on_shift_close` holds calls `on_day_closed` on the lifecycle hook.
    /// Answers whether it called the hook; no `ops_state` row answers `false`. Writes nothing itself.
    pub async fn close_day_if_last_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let _ = (&self.lifecycle, tx, branch, now);
        todo!("S1 #4136")
    }
}
