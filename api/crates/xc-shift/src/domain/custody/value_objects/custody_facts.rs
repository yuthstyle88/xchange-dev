//! Facts supplied to the custody machine.

use crate::domain::shift::value_objects::balances::Balances;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenShift {
    pub id: String,
    pub expected: Balances,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyFacts {
    pub open_shift: Option<OpenShift>,
    pub has_live_rate: bool,
    pub rates_updated: String,
    pub pending_transactions: Vec<String>,
    pub last_transaction: Option<String>,
    pub customer_known: bool,
    pub new_id: String,
}
