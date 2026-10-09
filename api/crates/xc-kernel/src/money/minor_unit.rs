//! Minor unit decimal places for a currency (0..=3), from ISO 4217.

use std::fmt;

use super::money::NumberError;

/// Number of fractional decimal places for a currency: 0..=3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MinorUnit(u8);

impl MinorUnit {
    pub const ZERO: MinorUnit = MinorUnit(0);
    pub const TWO: MinorUnit = MinorUnit(2);

    pub fn new(places: u8) -> Result<MinorUnit, NumberError> {
        if places > 3 {
            return Err(NumberError::Invalid);
        }
        Ok(MinorUnit(places))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl Default for MinorUnit {
    fn default() -> Self {
        Self::TWO
    }
}

impl fmt::Display for MinorUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
