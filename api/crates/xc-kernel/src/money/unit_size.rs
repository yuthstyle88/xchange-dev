//! Currency unit size (e.g. 100 for JPY, 1000 for KRW, 1 for USD).

use std::fmt;

use rust_decimal::Decimal;

use super::money::NumberError;

/// The selling unit size of a currency: greater than zero, at most 4 decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitSize(Decimal);

impl UnitSize {
    pub const ONE: UnitSize = UnitSize(Decimal::ONE);

    pub fn parse(text: &str) -> Result<UnitSize, NumberError> {
        if text.is_empty() {
            return Err(NumberError::Invalid);
        }
        if text.starts_with('.') || text.ends_with('.') {
            return Err(NumberError::Invalid);
        }
        let mut dot_count = 0;
        for b in text.bytes() {
            if b == b'.' {
                dot_count += 1;
                if dot_count > 1 {
                    return Err(NumberError::Invalid);
                }
            } else if !b.is_ascii_digit() {
                return Err(NumberError::Invalid);
            }
        }
        let frac_part = match text.split_once('.') {
            Some((_, frac)) => frac,
            None => "",
        };
        if frac_part.len() > 4 && frac_part[4..].bytes().any(|b| b != b'0') {
            return Err(NumberError::TooManyDecimals);
        }
        let val = Decimal::from_str_exact(text).map_err(|_| NumberError::Invalid)?;
        if val <= Decimal::ZERO {
            return Err(NumberError::Invalid);
        }
        Ok(UnitSize(val))
    }

    pub fn from_decimal(val: Decimal) -> Result<UnitSize, NumberError> {
        if val <= Decimal::ZERO {
            return Err(NumberError::Invalid);
        }
        if val.scale() > 4 {
            return Err(NumberError::TooManyDecimals);
        }
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
