//! Signed rate adjustment percentage at scale 4.

use std::fmt;

use rust_decimal::Decimal;

use super::money::NumberError;

/// Signed percentage adjustment with at most 4 decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RatePercent(Decimal);

impl RatePercent {
    pub const ZERO: RatePercent = RatePercent(Decimal::ZERO);

    pub fn parse(text: &str) -> Result<RatePercent, NumberError> {
        if text.is_empty() {
            return Err(NumberError::Invalid);
        }
        let rest = if let Some(stripped) = text.strip_prefix('-') {
            stripped
        } else if let Some(stripped) = text.strip_prefix('+') {
            stripped
        } else {
            text
        };

        if rest.is_empty() || rest.starts_with('.') || rest.ends_with('.') {
            return Err(NumberError::Invalid);
        }
        let mut dot_count = 0;
        for b in rest.bytes() {
            if b == b'.' {
                dot_count += 1;
                if dot_count > 1 {
                    return Err(NumberError::Invalid);
                }
            } else if !b.is_ascii_digit() {
                return Err(NumberError::Invalid);
            }
        }
        let frac_part = match rest.split_once('.') {
            Some((_, frac)) => frac,
            None => "",
        };
        if frac_part.len() > 4 && frac_part[4..].bytes().any(|b| b != b'0') {
            return Err(NumberError::TooManyDecimals);
        }
        let mut val = Decimal::from_str_exact(text).map_err(|_| NumberError::Invalid)?;
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
