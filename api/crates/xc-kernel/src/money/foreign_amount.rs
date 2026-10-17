//! Non-negative foreign currency amounts governed by the currency's minor unit.

use std::fmt;

use rust_decimal::Decimal;

use super::bounded::{Bound, Sign};
use super::currency_units::CurrencyUnits;
use super::money::{Money, NumberError};

/// A non-negative amount of a foreign currency conforming to its [`CurrencyUnits`].
///
/// Refuses more decimal places than the currency's `minor_unit` allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForeignAmount(Decimal);

impl ForeignAmount {
    pub fn parse(text: &str, units: &CurrencyUnits) -> Result<ForeignAmount, NumberError> {
        let val = Bound::numeric(20, units.minor_unit().get(), Sign::Positive)
            .at_most(Money::MAX_INPUT_UNITS)
            .parse(text)?;
        Ok(ForeignAmount(val))
    }

    pub fn from_decimal(val: Decimal) -> Self {
        ForeignAmount(val)
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for ForeignAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
