//! A signed amount in hundredths (e.g. stock deltas, count differences).

use std::fmt;

use super::money::{Money, NumberError};

/// A signed amount in hundredths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MoneyDelta(i128);

impl MoneyDelta {
    /// Zero delta.
    pub const ZERO: MoneyDelta = MoneyDelta(0);

    /// Constructs a delta from hundredths.
    pub fn from_minor(minor: i128) -> MoneyDelta {
        MoneyDelta(minor)
    }

    /// The delta as hundredths.
    pub fn minor(self) -> i128 {
        self.0
    }

    /// Adds a non-negative Money amount to this delta.
    pub fn plus(self, m: Money) -> MoneyDelta {
        MoneyDelta(self.0 + m.minor_i128())
    }

    /// Subtracts a non-negative Money amount from this delta.
    pub fn minus(self, m: Money) -> MoneyDelta {
        MoneyDelta(self.0 - m.minor_i128())
    }

    /// Adds another MoneyDelta to this delta.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: MoneyDelta) -> MoneyDelta {
        MoneyDelta(self.0 + other.0)
    }

    /// Adds a non-negative Money amount; `Err(NumberError::Invalid)` past `i128`.
    pub fn checked_plus(self, m: Money) -> Result<MoneyDelta, NumberError> {
        self.0
            .checked_add(m.minor_i128())
            .map(MoneyDelta)
            .ok_or(NumberError::Invalid)
    }

    /// Subtracts a non-negative Money amount; `Err(NumberError::Invalid)` past `i128`.
    pub fn checked_minus(self, m: Money) -> Result<MoneyDelta, NumberError> {
        self.0
            .checked_sub(m.minor_i128())
            .map(MoneyDelta)
            .ok_or(NumberError::Invalid)
    }

    /// Adds another MoneyDelta; `Err(NumberError::Invalid)` past `i128`.
    pub fn checked_add(self, other: MoneyDelta) -> Result<MoneyDelta, NumberError> {
        self.0
            .checked_add(other.0)
            .map(MoneyDelta)
            .ok_or(NumberError::Invalid)
    }

    /// True if the delta is negative.
    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// True if the delta is exactly zero.
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Converts back to Money if non-negative and inside what `Decimal` holds;
    /// `Err(NumberError::Invalid)` otherwise.
    pub fn to_money(self) -> Result<Money, NumberError> {
        Money::try_from_minor_i128(self.0)
    }
}

impl From<Money> for MoneyDelta {
    fn from(m: Money) -> Self {
        MoneyDelta(m.minor_i128())
    }
}

impl fmt::Display for MoneyDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let abs = self.0.unsigned_abs();
        let int_part = abs / 100;
        let frac_part = abs % 100;
        if self.0 < 0 {
            write!(f, "-{int_part}.{frac_part:02}")
        } else {
            write!(f, "{int_part}.{frac_part:02}")
        }
    }
}
