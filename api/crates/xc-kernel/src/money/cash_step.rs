//! The THB cash step: the smallest coin cash is rounded to at the counter
//! (`docs/design/blueprint/owner-set-values.md` row 5, §4). It is the Owner's
//! `pricing_rule_versions.thb_cash_step`, never a constant in code.

use std::fmt;

use rust_decimal::Decimal;

/// Why a value is not a cash step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CashStepError {
    /// The step is zero or negative.
    #[error("the cash step must be greater than zero")]
    NotPositive,
    /// The step has more than 2 decimal places (a satang is the smallest unit).
    #[error("the cash step has more than 2 decimal places")]
    TooManyDecimals,
    /// The step does not fit `numeric(6,2)`, the column that records it on every line.
    #[error("the cash step is too large")]
    TooLarge,
}

/// A THB cash step: greater than 0, at most 2 decimal places, and below 10,000
/// (`quote_lines.cash_step_thb` / `transaction_lines.cash_step_thb` are `numeric(6,2)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CashStep(Decimal);

impl CashStep {
    const LIMIT: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

    pub fn new(step: Decimal) -> Result<CashStep, CashStepError> {
        if step <= Decimal::ZERO {
            return Err(CashStepError::NotPositive);
        }
        let normalized = step.normalize();
        if normalized.scale() > 2 {
            return Err(CashStepError::TooManyDecimals);
        }
        if step >= Self::LIMIT {
            return Err(CashStepError::TooLarge);
        }
        let mut value = normalized;
        value.rescale(2);
        Ok(CashStep(value))
    }

    /// The step at scale 2.
    pub fn value(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for CashStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
