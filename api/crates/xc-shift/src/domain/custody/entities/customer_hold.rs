//! Customer hold entity for custody.

use crate::domain::custody::value_objects::custody_time::CustodyTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldStatus {
    Open,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerHold {
    pub id: String,
    pub customer_id: String,
    pub reason: String,
    pub by: String,
    pub at: CustodyTime,
    pub status: HoldStatus,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<CustodyTime>,
    pub note: Option<String>,
}
