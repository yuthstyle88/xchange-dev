//! Custody pages bounded read port.

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::CashException;
use crate::domain::custody::entities::customer_hold::CustomerHold;
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::entity::Entity;
use crate::domain::custody::value_objects::phase::Phase;

/// The `ops_state` columns for a branch, decoded using domain types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpsRow {
    pub phase: Phase,
    pub entity: Option<Entity>,
    pub target: Option<CashTarget>,
    pub cutoff: Option<Cutoff>,
    pub service_opened_at: Option<CustodyTime>,
}

/// The floor facts for a branch: the `ops_state` row, the user's latest open attendance record,
/// and the branch's OPEN shifts count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorFacts {
    pub ops: Option<OpsRow>,
    pub my_open_attendance: Option<Attendance>,
    pub open_shifts: usize,
}

/// Bounded, paged read port for custody history.
pub trait CustodyPages: Send + Sync + 'static {
    fn holds<'a>(
        &'a self,
        branch: &'a BranchId,
        before: Option<&'a str>,
        limit: u16,
    ) -> BoxFuture<'a, AppResult<Vec<CustomerHold>>>;

    fn attendance<'a>(
        &'a self,
        branch: &'a BranchId,
        only: Option<&'a str>,
        before: Option<&'a str>,
        limit: u16,
    ) -> BoxFuture<'a, AppResult<Vec<Attendance>>>;

    fn floor<'a>(
        &'a self,
        branch: &'a BranchId,
        me: &'a str,
    ) -> BoxFuture<'a, AppResult<FloorFacts>>;

    fn exceptions_between<'a>(
        &'a self,
        branch: &'a BranchId,
        from: i64,
        to: i64,
    ) -> BoxFuture<'a, AppResult<Vec<CashException>>>;

    fn open_holds<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<u64>>;
}
