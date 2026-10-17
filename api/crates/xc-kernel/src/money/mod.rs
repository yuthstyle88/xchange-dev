//! Money, rate, quote, delta and currency types.

mod bounded;
mod compat_rate;
mod currency_code;
mod currency_units;
mod customer_side;
mod exchange_arithmetic;
mod foreign_amount;
mod minor_unit;
#[allow(clippy::module_inception)]
mod money;
mod money_delta;
mod plain_decimal;
mod quote;
mod rate;
mod rate_percent;
mod side;
mod typed;
mod unit_size;

pub use bounded::{Bound, BoundError, IntBound, Sign};
pub use compat_rate::{CompatRate, CompatRateError};
pub use currency_code::{CurrencyCode, UnknownCurrency};
pub use currency_units::CurrencyUnits;
pub use customer_side::CustomerSide;
pub use exchange_arithmetic::{ArithmeticOverflow, ExchangeArithmetic};
pub use foreign_amount::ForeignAmount;
pub use minor_unit::MinorUnit;
pub use money::{Money, NumberError};
pub use money_delta::MoneyDelta;
pub use plain_decimal::plain_decimal;
pub use quote::{Quote, QuoteError};
pub use rate::Rate;
pub use rate_percent::RatePercent;
pub use side::Side;
pub use typed::{fixed2, fixed6, round_cents, typed_decimal};
pub use unit_size::UnitSize;
