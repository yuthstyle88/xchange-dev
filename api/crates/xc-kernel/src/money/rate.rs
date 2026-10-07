//! Baht per one unit of a foreign currency.

use std::fmt;

use rust_decimal::Decimal;

use super::money::{Money, NumberError, parse_decimal};

/// Baht per one unit of a foreign currency: greater than zero, `<= 100,000,000`,
/// at most four decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rate(Decimal);

impl Rate {
    /// Parses plain decimal notation (`"32.1"`, `"0.2195"`).
    pub fn parse(text: &str) -> Result<Rate, NumberError> {
        let val = parse_decimal(text, 4, true, Money::MAX_INPUT_UNITS)?;
        Ok(Rate(val))
    }

    /// The rate at exactly four decimal places, equal to and printing as [`fmt::Display`].
    pub fn value(self) -> Decimal {
        self.0
    }
}

/// Always four decimal places: `"32.1000"`.
impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = self.0;
        d.rescale(4);
        write!(f, "{d:.4}")
    }
}
