//! What one exchange is worth in baht.

use rust_decimal::RoundingStrategy;

use super::exchange_arithmetic::ExchangeArithmetic;
use super::foreign_amount::ForeignAmount;
use super::money::Money;
use super::rate::Rate;
use super::side::Side;
use super::unit_size::UnitSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuoteError {
    #[error("the amount must be greater than zero")]
    ZeroAmount,
    /// Buying only: the customer fee is larger than what the shop would pay out.
    #[error("the fee is larger than the purchase")]
    FeeExceedsPrincipal,
}

/// The baht figures of one exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    /// `amount × rate`, rounded half up to two places.
    pub principal: Money,
    /// Baht the shop takes in: `principal + fee` when selling, zero when buying.
    pub received: Money,
    /// Baht the shop pays out: `principal − fee` when buying, zero when selling.
    pub paid: Money,
}

impl Quote {
    pub fn compute(side: Side, amount: Money, rate: Rate, fee: Money) -> Result<Quote, QuoteError> {
        if amount.is_zero() {
            return Err(QuoteError::ZeroAmount);
        }

        let raw_principal = amount.value() * rate.value();
        let mut principal_dec =
            raw_principal.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        principal_dec.rescale(2);
        let principal = Money::from_decimal(principal_dec);

        match side {
            Side::Buy => {
                if fee > principal {
                    return Err(QuoteError::FeeExceedsPrincipal);
                }
                let mut paid_dec = principal.value() - fee.value();
                paid_dec.rescale(2);
                Ok(Quote {
                    principal,
                    received: Money::ZERO,
                    paid: Money::from_decimal(paid_dec),
                })
            }
            Side::Sell => {
                let mut received_dec = principal.value() + fee.value();
                received_dec.rescale(2);
                Ok(Quote {
                    principal,
                    received: Money::from_decimal(received_dec),
                    paid: Money::ZERO,
                })
            }
        }
    }

    pub fn per_unit(
        side: Side,
        amount: ForeignAmount,
        rate: Rate,
        unit: UnitSize,
        fee: Money,
    ) -> Result<Quote, QuoteError> {
        if amount.value().is_zero() {
            return Err(QuoteError::ZeroAmount);
        }

        let principal = ExchangeArithmetic::thb(amount, rate, unit);

        match side {
            Side::Buy => {
                if fee > principal {
                    return Err(QuoteError::FeeExceedsPrincipal);
                }
                let mut paid_dec = principal.value() - fee.value();
                paid_dec.rescale(2);
                Ok(Quote {
                    principal,
                    received: Money::ZERO,
                    paid: Money::from_decimal(paid_dec),
                })
            }
            Side::Sell => {
                let mut received_dec = principal.value() + fee.value();
                received_dec.rescale(2);
                Ok(Quote {
                    principal,
                    received: Money::from_decimal(received_dec),
                    paid: Money::ZERO,
                })
            }
        }
    }
}
