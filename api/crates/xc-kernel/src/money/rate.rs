//! Baht per unit of a foreign currency.

use std::fmt;

use rust_decimal::Decimal;

use super::money::{Money, NumberError};

/// Baht per unit of a foreign currency: greater than zero, `<= 100,000,000`,
/// up to 8 decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rate(Decimal);

impl Rate {
    /// Parses plain decimal notation (`"32.1"`, `"0.2195"`, `"25.12345678"`).
    /// Accepts up to 8 decimal places. A 9th non-zero place is [`NumberError::TooManyDecimals`].
    pub fn parse(text: &str) -> Result<Rate, NumberError> {
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
        if frac_part.len() > 8 && frac_part[8..].bytes().any(|b| b != b'0') {
            return Err(NumberError::TooManyDecimals);
        }
        let val = Decimal::from_str_exact(text).map_err(|_| NumberError::Invalid)?;
        if val.is_zero() {
            return Err(NumberError::Invalid);
        }
        if val > Decimal::from(Money::MAX_INPUT_UNITS) {
            return Err(NumberError::Invalid);
        }

        let sig_len = frac_part[..std::cmp::min(frac_part.len(), 8)]
            .trim_end_matches('0')
            .len();
        let target_scale = (sig_len as u32).clamp(4, 8);
        let mut rescaled = val;
        rescaled.rescale(target_scale);
        Ok(Rate(rescaled))
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
