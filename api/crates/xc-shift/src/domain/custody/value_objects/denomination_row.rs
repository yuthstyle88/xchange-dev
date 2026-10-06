//! Denomination row for count sheets.

use xc_kernel::money::{CurrencyCode, Money};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DenominationRow {
    pub currency: CurrencyCode,
    pub denomination: Money,
    pub count: u32,
}
