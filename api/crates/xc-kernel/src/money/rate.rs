//! Baht per unit of a foreign currency.

use std::fmt;

use rust_decimal::Decimal;

use super::bounded::{Bound, Sign};
use super::money::{Money, NumberError};

/// Baht per unit of a foreign currency: greater than zero, `<= 100,000,000`,
/// up to 8 decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rate(Decimal);

impl Rate {
    /// Parses plain decimal notation (`"32.1"`, `"0.2195"`, `"25.12345678"`).
    /// Accepts up to 8 decimal places. A 9th non-zero place is [`NumberError::TooManyDecimals`].
    pub fn parse(text: &str) -> Result<Rate, NumberError> {
        let mut val = Bound::numeric(20, 8, Sign::Positive)
            .at_most(Money::MAX_INPUT_UNITS)
            .parse(text)?;
        let target_scale = val.normalize().scale().clamp(4, 8);
        val.rescale(target_scale);
        Ok(Rate(val))
    }

    /// The rate at between 4 and 8 decimal places, equal to and printing as [`fmt::Display`].
    pub fn value(self) -> Decimal {
        self.0
    }
}

/// At least 4 and at most 8 decimal places: `"32.1000"`, `"0.02152"`, `"25.12345678"`.
impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
