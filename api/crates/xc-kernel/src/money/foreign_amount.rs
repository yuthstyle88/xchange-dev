//! Non-negative foreign currency amounts governed by the currency's minor unit.

use std::fmt;

use rust_decimal::Decimal;

use super::bounded::{Bound, Sign};
use super::currency_units::CurrencyUnits;
use super::money::{Money, NumberError};

/// A non-negative amount of a foreign currency conforming to its [`CurrencyUnits`].
///
/// Refuses more decimal places than the currency's `minor_unit` allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForeignAmount(Decimal);

impl ForeignAmount {
    pub const ZERO: ForeignAmount = ForeignAmount(Decimal::ZERO);

    pub fn parse(text: &str, units: &CurrencyUnits) -> Result<ForeignAmount, NumberError> {
        let val = Bound::numeric(20, units.minor_unit().get(), Sign::Positive)
            .at_most(Money::MAX_INPUT_UNITS)
            .parse(text)?;
        Ok(ForeignAmount(val))
    }

    pub fn checked_from_decimal(val: Decimal) -> Result<ForeignAmount, NumberError> {
        if val.is_sign_negative() && !val.is_zero() {
            return Err(NumberError::Invalid);
        }
        let mut d = val;
        if d.scale() > 4 {
            d = d.normalize();
            if d.scale() > 4 {
                return Err(NumberError::TooManyDecimals);
            }
        }
        Ok(ForeignAmount(d))
    }

    pub fn from_decimal(val: Decimal) -> Self {
        ForeignAmount(val)
    }

    pub fn value(self) -> Decimal {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }

    pub fn checked_plus(self, other: ForeignAmount) -> Result<ForeignAmount, NumberError> {
        self.0
            .checked_add(other.0)
            .ok_or(NumberError::Invalid)
            .and_then(Self::checked_from_decimal)
    }

    pub fn checked_minus(self, other: ForeignAmount) -> Result<ForeignAmount, NumberError> {
        self.0
            .checked_sub(other.0)
            .ok_or(NumberError::Invalid)
            .and_then(Self::checked_from_decimal)
    }

    pub fn fits(self, units: &CurrencyUnits) -> bool {
        self.0.normalize().scale() <= u32::from(units.minor_unit().get())
    }
}

impl fmt::Display for ForeignAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
