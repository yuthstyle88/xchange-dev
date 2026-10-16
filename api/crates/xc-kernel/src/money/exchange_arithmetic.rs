//! The single arithmetic for converting between foreign amounts and THB.

use rust_decimal::RoundingStrategy;

use super::currency_units::CurrencyUnits;
use super::foreign_amount::ForeignAmount;
use super::money::Money;
use super::rate::Rate;
use super::unit_size::UnitSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("arithmetic overflow")]
pub struct ArithmeticOverflow;

pub struct ExchangeArithmetic;

impl ExchangeArithmetic {
    /// THB for `amount` foreign at `rate` per `unit`: amount × rate ÷ unit_size,
    /// rounded HALF_UP (away from zero) to 2 places once, at the end (§05).
    pub fn thb(amount: ForeignAmount, rate: Rate, unit: UnitSize) -> Money {
        let raw = (amount.value() * rate.value()) / unit.value();
        let mut rounded = raw.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        rounded.rescale(2);
        Money::from_decimal(rounded)
    }

    /// Checked form of [`Self::thb`]: returns `Err(ArithmeticOverflow)` instead of panicking
    /// when `amount × rate` or `÷ unit_size` overflows `Decimal`.
    pub fn try_thb(
        amount: ForeignAmount,
        rate: Rate,
        unit: UnitSize,
    ) -> Result<Money, ArithmeticOverflow> {
        let raw = amount
            .value()
            .checked_mul(rate.value())
            .and_then(|product| product.checked_div(unit.value()))
            .ok_or(ArithmeticOverflow)?;
        let mut rounded = raw.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        rounded.rescale(2);
        Ok(Money::from_decimal(rounded))
    }

    /// The most foreign `thb` buys at `rate`: thb × unit_size ÷ rate, truncated
    /// to the currency's minor unit (the kiosk's SELL by baht).
    pub fn foreign_for_thb(thb: Money, rate: Rate, units: &CurrencyUnits) -> ForeignAmount {
        let raw = (thb.value() * units.unit_size().value()) / rate.value();
        let minor = units.minor_unit().get() as u32;
        let truncated = raw.round_dp_with_strategy(minor, RoundingStrategy::ToZero);
        ForeignAmount::from_decimal(truncated)
    }

    /// Checked form of [`Self::foreign_for_thb`]: returns `Err(ArithmeticOverflow)` instead of
    /// panicking when `thb × unit_size` or `÷ rate` overflows `Decimal`.
    pub fn try_foreign_for_thb(
        thb: Money,
        rate: Rate,
        units: &CurrencyUnits,
    ) -> Result<ForeignAmount, ArithmeticOverflow> {
        let raw = thb
            .value()
            .checked_mul(units.unit_size().value())
            .and_then(|product| product.checked_div(rate.value()))
            .ok_or(ArithmeticOverflow)?;
        let minor = units.minor_unit().get() as u32;
        let truncated = raw.round_dp_with_strategy(minor, RoundingStrategy::ToZero);
        Ok(ForeignAmount::from_decimal(truncated))
    }
}
