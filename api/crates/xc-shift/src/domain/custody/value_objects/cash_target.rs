//! Custody cash target definition.

use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::shift::value_objects::balances::Balances;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashTarget {
    pub amounts: Balances,
    pub by: String,
    pub at: CustodyTime,
}
