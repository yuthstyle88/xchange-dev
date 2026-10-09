//! Non-negative foreign currency amounts governed by the currency's minor unit.

use std::fmt;

use rust_decimal::Decimal;

use super::currency_units::CurrencyUnits;
use super::money::{Money, NumberError};

/// A non-negative amount of a foreign currency conforming to its [`CurrencyUnits`].
///
/// Refuses more decimal places than the currency's `minor_unit` allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForeignAmount(Decimal);

impl ForeignAmount {
    pub fn parse(text: &str, units: &CurrencyUnits) -> Result<ForeignAmount, NumberError> {
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
        let max_dp = units.minor_unit().get() as usize;
        if frac_part.len() > max_dp && frac_part[max_dp..].bytes().any(|b| b != b'0') {
            return Err(NumberError::TooManyDecimals);
        }
        let val = Decimal::from_str_exact(text).map_err(|_| NumberError::Invalid)?;
        if val.is_zero() {
            return Err(NumberError::Invalid);
        }
        if val > Decimal::from(Money::MAX_INPUT_UNITS) {
            return Err(NumberError::Invalid);
        }
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
