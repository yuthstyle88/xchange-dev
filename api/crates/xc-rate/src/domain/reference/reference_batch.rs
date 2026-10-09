//! Reference batch domain types.

use xc_kernel::money::{CurrencyCode, Rate, UnitSize};

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceLine {
    pub currency: CurrencyCode,
    pub unit_size: UnitSize,
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
}

impl ReferenceLine {
    pub fn new(
        currency: CurrencyCode,
        unit_size: UnitSize,
        buy: Option<Rate>,
        sell: Option<Rate>,
    ) -> Self {
        Self {
            currency,
            unit_size,
            buy,
            sell,
        }
    }
}
