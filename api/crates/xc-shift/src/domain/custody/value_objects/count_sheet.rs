//! Count sheet definition.

use crate::domain::custody::value_objects::denomination_row::DenominationRow;
use crate::domain::shift::value_objects::balances::Balances;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountSheet {
    pub totals: Balances,
    pub denominations: Vec<DenominationRow>,
}

impl CountSheet {
    pub fn empty() -> CountSheet {
        CountSheet {
            totals: Balances::empty(),
            denominations: Vec::new(),
        }
    }
}
