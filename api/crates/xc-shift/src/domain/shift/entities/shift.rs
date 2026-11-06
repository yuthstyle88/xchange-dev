//! Shift entity.

use chrono::{DateTime, NaiveDate, Utc};
use xc_kernel::ids::BranchId;

use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::StockDelta;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shift {
    pub id: String,
    pub branch_id: BranchId,
    pub status: ShiftStatus,
    pub opened_at: DateTime<Utc>,
    pub opened_by: String,
    pub opening: Balances,
    pub expected: Balances,
    pub closed_at: Option<DateTime<Utc>>,
    pub closed_by: Option<String>,
    pub counted: Option<Balances>,
    pub difference: Option<StockDelta>,
    pub note: String,
    pub open_request_id: String,
    pub close_request_id: Option<String>,
    /// The template the teller opened as (configurable-shifts.md §2.3); `None` before the feature.
    pub template_id: Option<String>,
    /// The template's name at opening.
    pub template_name: Option<String>,
    pub business_date: NaiveDate,
}
