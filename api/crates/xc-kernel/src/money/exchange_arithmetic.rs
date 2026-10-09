//! The single arithmetic for converting between foreign amounts and THB.

use rust_decimal::RoundingStrategy;

use super::currency_units::CurrencyUnits;
use super::foreign_amount::ForeignAmount;
use super::money::Money;
use super::rate::Rate;
use super::unit_size::UnitSize;

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

    /// The most foreign `thb` buys at `rate`: thb × unit_size ÷ rate, truncated
    /// to the currency's minor unit (the kiosk's SELL by baht).
    pub fn foreign_for_thb(thb: Money, rate: Rate, units: &CurrencyUnits) -> ForeignAmount {
        let raw = (thb.value() * units.unit_size().value()) / rate.value();
        let minor = units.minor_unit().get() as u32;
        let truncated = raw.round_dp_with_strategy(minor, RoundingStrategy::ToZero);
        ForeignAmount::from_decimal(truncated)
    }
}
