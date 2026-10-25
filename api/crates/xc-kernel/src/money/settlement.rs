//! Cash THB rounding at the counter (`docs/design/blueprint/quote-transaction.md` §13.2).

use rust_decimal::Decimal;

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
}

/// The settled THB and cash rounding of one exchange line (§13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settlement {
    settle: Money,
    rounding: Decimal,
}

impl Settlement {
    /// The smallest THB coin at the counter, 0.25 baht (§13.2).
    pub const THB_CASH_STEP: Decimal = Decimal::from_parts(25, 0, 0, false, 2);

    /// Computes `settle` and `rounding = settle − u` at scale 2, rounding cash in the customer's favour (§13.2).
    pub fn of(
        side: Side,
        thb: Money,
        fee: Money,
        cash: bool,
    ) -> Result<Settlement, SettlementError> {
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

        let rem = if cash {
            u % Self::THB_CASH_STEP
        } else {
            Decimal::ZERO
        };

        let mut rounding = if !cash || rem.is_zero() {
            Decimal::ZERO
        } else {
            match side {
                Side::Buy => Self::THB_CASH_STEP - rem,
                Side::Sell => -rem,
            }
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
}
