//! `BranchClosure` (design §5, `branch_closures`): a date the branch takes no bookings (B2).

use chrono::NaiveDate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchClosure {
    pub closed_on: NaiveDate,
    pub reason: String,
}
