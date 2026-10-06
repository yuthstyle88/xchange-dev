//! Custody actor session info.

use crate::domain::custody::value_objects::custody_role::CustodyRole;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyActor {
    pub id: String,
    pub employee_id: Option<String>,
    pub role: CustodyRole,
}
