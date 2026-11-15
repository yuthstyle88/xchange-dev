//! Cash THB rounding at the counter (`docs/design/blueprint/quote-transaction.md` §13.2). The step is
//! the Owner's `thb_cash_step` (`docs/design/blueprint/owner-set-values.md` row 5), passed in by the caller.

use rust_decimal::Decimal;

use super::cash_step::CashStep;
use super::money::Money;
use super::side::Side;

/// Why a THB settlement could not be computed (§13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SettlementError {
    /// Buying only: the fee exceeds the principal (`fee > thb`, §13.2).
    #[error("the fee is larger than the purchase")]
    FeeAbovePrincipal,
    /// Cash selling only: the rounded amount due would be 0.00 (§13.2).
    #[error("amount is below the smallest coin")]
    BelowSmallestCoin,
    /// The settlement or rounding addition overflowed `Decimal` (§13.2).
    #[error("settlement overflow")]
    Overflow,
    /// A cash settlement with no cash step: the Owner has not set `thb_cash_step` (row 5, G-9).
    #[error("the THB cash step is not set")]
    CashStepNotSet,
}

/// The settled THB and cash rounding of one exchange line (§13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settlement {
    settle: Money,
    rounding: Decimal,
    cash_step: Option<CashStep>,
}

impl Settlement {
    /// Computes `settle` and `rounding = settle − u` at scale 2, rounding cash in the customer's favour to a
    /// multiple of `step` (§13.2). Cash needs a step (`CashStepNotSet` without one); a non-cash settlement
    /// ignores `step` and never rounds.
    pub fn of(
        side: Side,
        thb: Money,
        fee: Money,
        cash: bool,
        step: Option<CashStep>,
    ) -> Result<Settlement, SettlementError> {
        let step = if cash {
            Some(step.ok_or(SettlementError::CashStepNotSet)?)
        } else {
            None
        };
        let mut u = match side {
            Side::Buy => {
                if fee > thb {
                    return Err(SettlementError::FeeAbovePrincipal);
                }
                thb.value() - fee.value()
            }
            Side::Sell => thb
                .value()
                .checked_add(fee.value())
                .ok_or(SettlementError::Overflow)?,
        };
        u.rescale(2);
        if u.scale() != 2 {
            return Err(SettlementError::Overflow);
        }

        let rem = match step {
            Some(step) => u % step.value(),
            None => Decimal::ZERO,
        };

        let mut rounding = match step {
            Some(step) if !rem.is_zero() => match side {
                Side::Buy => step.value() - rem,
                Side::Sell => -rem,
            },
            _ => Decimal::ZERO,
        };
        rounding.rescale(2);

        let mut settle_dec = u.checked_add(rounding).ok_or(SettlementError::Overflow)?;
        settle_dec.rescale(2);
        if settle_dec.scale() != 2 {
            return Err(SettlementError::Overflow);
        }
        if cash && side == Side::Sell && settle_dec.is_zero() {
            return Err(SettlementError::BelowSmallestCoin);
        }

        Ok(Settlement {
            settle: Money::from_decimal(settle_dec),
            rounding,
            cash_step: step,
        })
    }

    /// The THB amount that changes hands (§13.2).
    pub fn settle(&self) -> Money {
        self.settle
    }

    /// The signed rounding adjustment `settle − u` at scale 2 (§13.2).
    pub fn rounding(&self) -> Decimal {
        self.rounding
    }

    /// The step a cash settlement was rounded to; `None` for a non-cash one (row 5: every line records it).
    pub fn cash_step(&self) -> Option<CashStep> {
        self.cash_step
    }
}
