//! Signed rate adjustment percentage at scale 4.

use std::fmt;

use rust_decimal::Decimal;

use super::bounded::{Bound, Sign};
use super::money::NumberError;

/// Signed percentage adjustment with at most 4 decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RatePercent(Decimal);

impl RatePercent {
    pub const ZERO: RatePercent = RatePercent(Decimal::ZERO);

    pub fn parse(text: &str) -> Result<RatePercent, NumberError> {
        let mut val = match text.strip_prefix('+') {
            Some(rest) => Bound::numeric(9, 4, Sign::NonNegative).parse(rest)?,
            None => Bound::numeric(9, 4, Sign::Signed).parse(text)?,
        };
        val.rescale(4);
        Ok(RatePercent(val))
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for RatePercent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = self.0;
        d.rescale(4);
        write!(f, "{d:.4}")
    }
}
