//! `BranchServiceStatus` (design §4.1 `branch_service_status`, §5, Q8, Q9, Q13, Q14).

use chrono::{DateTime, Utc};
use xc_kernel::ids::BranchId;

use super::wait_estimator::WaitEstimate;

/// One row of `branch_service_status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchServiceStatus {
    pub branch_id: BranchId,
    pub organization_id: String,
    /// Closed: no new ticket is issued; existing tickets are still served (Q8).
    pub queue_open: bool,
    /// The branch's ACTIVE counters (Q7, owner Q-A4).
    pub active_counters: i32,
    /// Raised with the branch version (Q13).
    pub queue_version: i64,
    /// `None` = unknown: no sample, or no active counter (Q9). The three columns are NULL together.
    pub estimate: Option<WaitEstimate>,
    /// When the estimate was computed, also when it is unknown (§17 *"as_of"*).
    pub estimate_as_of: Option<DateTime<Utc>>,
}
