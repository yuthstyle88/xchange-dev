//! Amounts and rates as exact decimals.
//!
//! v2 held these as IEEE doubles and rounded with `Math.round((n + EPSILON) * 100) / 100`.
//! Here nothing is ever a float.

use std::fmt;

use rust_decimal::Decimal;

use super::bounded::{Bound, BoundError, Sign};

/// Why a number typed by a person or sent by a screen was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NumberError {
    /// Not a plain decimal number, negative, or above the limit.
    #[error("not a valid amount or rate")]
    Invalid,
    /// A non-zero digit beyond the allowed decimal places.
    #[error("too many decimal places")]
    TooManyDecimals,
}

/// A non-negative amount of one currency with two decimal places.
///
/// A figure typed by a person ([`Money::parse`]) may not exceed 100,000,000. A figure
/// the system computed (a principal, a balance) may: v2 stores a principal of
/// 3,209,999,999.68 for 99,999,999.99 × 32.1, and so must this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Money(Decimal);

impl Money {
    /// The largest amount a person may enter, in whole units.
    pub const MAX_INPUT_UNITS: i64 = 100_000_000;

    pub const ZERO: Money = Money(Decimal::ZERO);

    /// Parses plain decimal notation (`"100"`, `"100.5"`, `"0.07"`), `0 ..= 100,000,000`.
    pub fn parse(text: &str) -> Result<Money, NumberError> {
        let mut val = Bound::numeric(20, 2, Sign::NonNegative)
            .at_most(Self::MAX_INPUT_UNITS)
            .parse(text)?;
        val.rescale(2);
        Ok(Money(val))
    }

    /// The amount as hundredths (satang, cents).
    pub fn minor(self) -> i64 {
        let mut d = self.0;
        d.rescale(2);
        d.mantissa() as i64
    }

    fn hundredths(self) -> Option<i128> {
        let mut d = self.0;
        d.rescale(2);
        if d.scale() == 2 {
            Some(d.mantissa())
        } else {
            None
        }
    }

    /// The amount as hundredths with no 64-bit limit.
    pub fn minor_i128(self) -> i128 {
        self.hundredths()
            .expect("Money figure is not representable at 2 decimal places")
    }

    /// The amount as hundredths (satang, cents), or `Err(NumberError::Invalid)` when it does not
    /// fit in `i64`.
    pub fn try_minor(self) -> Result<i64, NumberError> {
        self.hundredths()
            .and_then(|m| i64::try_from(m).ok())
            .ok_or(NumberError::Invalid)
    }

    /// The inverse of [`Money::minor`]; no upper cap. `Err(Invalid)` when negative.
    pub fn from_minor(minor: i64) -> Result<Money, NumberError> {
        if minor < 0 {
            return Err(NumberError::Invalid);
        }
        Ok(Money(Decimal::new(minor, 2)))
    }

    /// Hundredths as an amount: `Err(NumberError::Invalid)` when negative or past what
    /// `Decimal` holds at scale 2.
    pub fn try_from_minor_i128(minor: i128) -> Result<Money, NumberError> {
        if minor < 0 {
            return Err(NumberError::Invalid);
        }
        Decimal::try_from_i128_with_scale(minor, 2)
            .map(Money)
            .map_err(|_| NumberError::Invalid)
    }

    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }

    pub(crate) fn value(self) -> Decimal {
        self.0
    }

    pub(crate) fn from_decimal(d: Decimal) -> Self {
        Money(d)
    }
}

/// Always two decimal places and no grouping: `"3210.00"`.
impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = self.0;
        d.rescale(2);
        write!(f, "{d:.2}")
    }
}

impl From<BoundError> for NumberError {
    fn from(err: BoundError) -> Self {
        match err {
            BoundError::TooManyDecimals { .. } => NumberError::TooManyDecimals,
            BoundError::Syntax | BoundError::OutOfRange | BoundError::Sign => NumberError::Invalid,
        }
    }
}
