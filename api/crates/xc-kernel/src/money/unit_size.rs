//! Currency unit size (e.g. 100 for JPY, 1000 for KRW, 1 for USD).

use std::fmt;

use rust_decimal::Decimal;

use super::bounded::{Bound, Sign};
use super::money::NumberError;

/// The selling unit size of a currency: greater than zero, at most 4 decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitSize(Decimal);

impl UnitSize {
    pub const ONE: UnitSize = UnitSize(Decimal::ONE);

    pub fn parse(text: &str) -> Result<UnitSize, NumberError> {
        let val = Bound::numeric(20, 4, Sign::Positive).parse(text)?;
        Ok(UnitSize(val))
    }

    pub fn from_decimal(val: Decimal) -> Result<UnitSize, NumberError> {
        if val <= Decimal::ZERO {
            return Err(NumberError::Invalid);
        }
        if val.scale() > 4 {
            return Err(NumberError::TooManyDecimals);
        }
        let _ = Bound::numeric(20, 4, Sign::Positive).parse(&val.to_string())?;
        Ok(UnitSize(val))
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

impl Default for UnitSize {
    fn default() -> Self {
        Self::ONE
    }
}

impl fmt::Display for UnitSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
