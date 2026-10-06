//! Custody signature definition.

use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub employee_id: String,
    pub role: CustodyRole,
    pub at: CustodyTime,
}
