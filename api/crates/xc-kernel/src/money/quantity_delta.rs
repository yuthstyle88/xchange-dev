//! A signed foreign quantity at up to four decimal places, the signed twin of [`ForeignAmount`].

use std::fmt;

use rust_decimal::Decimal;

use super::currency_units::CurrencyUnits;
use super::foreign_amount::ForeignAmount;
use super::money::NumberError;

/// A signed foreign quantity at up to four decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuantityDelta(Decimal);

impl QuantityDelta {
    /// Zero delta.
    pub const ZERO: QuantityDelta = QuantityDelta(Decimal::ZERO);

    /// Constructs a delta from a decimal of scale ≤ 4; refuses scale > 4 with [`NumberError::TooManyDecimals`].
    pub fn from_decimal(val: Decimal) -> Result<QuantityDelta, NumberError> {
        let mut d = val;
        if d.scale() > 4 {
            d = d.normalize();
            if d.scale() > 4 {
                return Err(NumberError::TooManyDecimals);
            }
        }
        Ok(QuantityDelta(d))
    }

    /// The delta as a decimal.
    pub fn value(self) -> Decimal {
        self.0
    }

    /// Adds a non-negative [`ForeignAmount`] to this delta; `Err(NumberError::Invalid)` on overflow.
    pub fn checked_plus(self, amount: ForeignAmount) -> Result<QuantityDelta, NumberError> {
        let b = amount.value();
        let r = self.0.checked_add(b).ok_or(NumberError::Invalid)?;
        if r.scale() < self.0.scale().max(b.scale()) || r.checked_sub(b) != Some(self.0) {
            return Err(NumberError::Invalid);
        }
        Self::from_decimal(r)
    }

    /// Subtracts a non-negative [`ForeignAmount`] from this delta; `Err(NumberError::Invalid)` on overflow.
    pub fn checked_minus(self, amount: ForeignAmount) -> Result<QuantityDelta, NumberError> {
        let b = amount.value();
        let r = self.0.checked_sub(b).ok_or(NumberError::Invalid)?;
        if r.scale() < self.0.scale().max(b.scale()) || r.checked_add(b) != Some(self.0) {
            return Err(NumberError::Invalid);
        }
        Self::from_decimal(r)
    }

    /// Adds another [`QuantityDelta`]; `Err(NumberError::Invalid)` on overflow.
    pub fn checked_add(self, other: QuantityDelta) -> Result<QuantityDelta, NumberError> {
        let b = other.0;
        let r = self.0.checked_add(b).ok_or(NumberError::Invalid)?;
        if r.scale() < self.0.scale().max(b.scale()) || r.checked_sub(b) != Some(self.0) {
            return Err(NumberError::Invalid);
        }
        Self::from_decimal(r)
    }

    /// True if the delta is negative.
    pub fn is_negative(self) -> bool {
        self.0.is_sign_negative() && !self.0.is_zero()
    }

    /// True if the delta is exactly zero.
    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }

    /// Converts back to [`ForeignAmount`] if non-negative and scale ≤ 4.
    pub fn to_amount(self) -> Result<ForeignAmount, NumberError> {
        ForeignAmount::checked_from_decimal(self.0)
    }

    /// True when the normalized scale is at most the currency's `minor_unit`.
    pub fn fits(self, units: &CurrencyUnits) -> bool {
        self.0.normalize().scale() <= u32::from(units.minor_unit().get())
    }
}

impl From<ForeignAmount> for QuantityDelta {
    fn from(amount: ForeignAmount) -> Self {
        QuantityDelta(amount.value())
    }
}

impl fmt::Display for QuantityDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
