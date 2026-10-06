//! Custody entity definition.

use crate::domain::custody::value_objects::custody_time::CustodyTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub company: String,
    pub site: String,
    pub confirmed_by: String,
    pub at: CustodyTime,
}
