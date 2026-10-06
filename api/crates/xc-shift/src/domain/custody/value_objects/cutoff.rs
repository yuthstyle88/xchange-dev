//! Custody cutoff definition.

use crate::domain::custody::value_objects::custody_time::CustodyTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cutoff {
    pub at: CustodyTime,
    pub by: String,
    pub shift_id: String,
    pub last_ref: Option<String>,
    pub pending: Vec<String>,
}
