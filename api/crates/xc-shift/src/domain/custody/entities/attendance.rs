//! Attendance entity for custody.

use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendance {
    pub id: String,
    pub employee_id: String,
    pub role: CustodyRole,
    pub clock_in: CustodyTime,
    pub clock_out: Option<CustodyTime>,
}
