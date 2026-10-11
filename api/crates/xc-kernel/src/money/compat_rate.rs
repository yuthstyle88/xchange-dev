//! Compatibility rate calculation (per 1 unit) for v2-compatible surfaces.

use std::fmt;

use rust_decimal::Decimal;

use super::money::{Money, NumberError};
use super::rate::Rate;
use super::unit_size::UnitSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatRateError {
    NotPowerOfTen,
    InvalidScale,
}

impl fmt::Display for CompatRateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPowerOfTen => write!(f, "unit_size is not a power of 10"),
            Self::InvalidScale => write!(f, "resulting scale exceeds decimal limit"),
        }
    }
}

impl std::error::Error for CompatRateError {}

/// Calculates the v2-compatible rate per 1 unit from a rate per `unit_size`.
pub struct CompatRate;

impl CompatRate {
    /// Turns a rate per `unit_size` into a rate per 1 unit.
    ///
    /// The division is exact because `unit_size` must be a power of 10,
    /// so it only shifts the decimal point without rounding.
    /// A `unit_size` that is not a power of 10 is refused with an error.
    pub fn per_one(rate: Rate, unit_size: UnitSize) -> Result<Decimal, CompatRateError> {
        let norm_unit = unit_size.value().normalize();
        if norm_unit.scale() != 0 || norm_unit <= Decimal::ZERO {
            return Err(CompatRateError::NotPowerOfTen);
        }
        let mut m = norm_unit.mantissa();
        let mut k: u32 = 0;
        while m > 0 && m % 10 == 0 {
            m /= 10;
            k += 1;
        }
        if m != 1 {
            return Err(CompatRateError::NotPowerOfTen);
        }

        let mut val = rate.value();
        let new_scale = val
            .scale()
            .checked_add(k)
            .ok_or(CompatRateError::InvalidScale)?;
        val.set_scale(new_scale)
            .map_err(|_| CompatRateError::InvalidScale)?;
        Ok(val.normalize())
    }

    /// Parses a v2-compatible rate per 1 unit text from the wire (up to 11 decimal places).
    ///
    /// Accepts up to 11 decimal places. A 12th non-zero place is [`NumberError::TooManyDecimals`].
    /// Trailing zeros past the 11th place are accepted.
    /// Numbers exceeding [`Money::MAX_INPUT_UNITS`], zero, negative, or not plain decimal are [`NumberError::Invalid`].
    pub fn parse_per_one(text: &str) -> Result<Decimal, NumberError> {
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
        if frac_part.len() > 11 && frac_part[11..].bytes().any(|b| b != b'0') {
            return Err(NumberError::TooManyDecimals);
        }
        let val = Decimal::from_str_exact(text).map_err(|_| NumberError::Invalid)?;
        if val.is_zero() || val.is_sign_negative() {
            return Err(NumberError::Invalid);
        }
        if val > Decimal::from(Money::MAX_INPUT_UNITS) {
            return Err(NumberError::Invalid);
        }
        Ok(val)
    }
}
