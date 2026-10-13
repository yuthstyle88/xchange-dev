//! xc-marketing's own scope and period (ruling 3 on #1406): executive's follow-up converts its `&[BranchId]` and
//! `ExecutiveFilter` into these.

use chrono::NaiveDate;
use xc_kernel::ids::BranchId;

/// The branches whose exchanges count: a redemption counts when its transaction's `branch_id` is one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformanceScope {
    pub branches: Vec<BranchId>,
}

impl PerformanceScope {
    pub fn new(branches: Vec<BranchId>) -> Self {
        Self { branches }
    }
}

/// Business days `from ..= to`, both inclusive, in each branch's own `branches.timezone` (as executive's facts read
/// a period). A redemption counts when its `redeemed_at`, as a date in its transaction's branch timezone, is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformancePeriod {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl PerformancePeriod {
    pub fn new(from: NaiveDate, to: NaiveDate) -> Self {
        Self { from, to }
    }
}
