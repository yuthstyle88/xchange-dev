//! Rate proposal logic: draft line + reference + policy -> proposed rates.

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyUnits, Rate};

use crate::domain::draft::rate_draft::{DraftLine, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::reference::reference_batch::ReferenceLine;

#[derive(Debug, Clone, PartialEq)]
pub struct ProposedLine {
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
}

pub struct RateProposal;

impl RateProposal {
    pub fn of(
        line: &DraftLine,
        reference: Option<&ReferenceLine>,
        policy: Option<&RatePolicy>,
        units: &CurrencyUnits,
    ) -> Result<ProposedLine, RateEngineError> {
        let has_manual =
            matches!(line.buy, LineSide::Manual(_)) || matches!(line.sell, LineSide::Manual(_));
        if has_manual {
            match &line.reason {
                Some(r) if !r.trim().is_empty() => (),
                _ => {
                    return Err(RateEngineError::ValidationFailed(
                        "a MANUAL side must carry a non-empty reason".into(),
                    ));
                }
            }
        }

        let buy = match &line.buy {
            LineSide::Manual(rate) => Some(*rate),
            LineSide::Auto => match (policy, reference) {
                (Some(pol), Some(ref_line)) => {
                    if let Some(ref_buy) = ref_line.buy {
                        let base = ref_buy.value() * units.unit_size().value()
                            / ref_line.unit_size.value();
                        let adjust =
                            Decimal::ONE + pol.buy_adjust_pct().value() / Decimal::from(100);
                        let raw = base * adjust;
                        let rounded = raw.round_dp_with_strategy(
                            8,
                            rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                        );
                        if rounded <= Decimal::ZERO {
                            return Err(RateEngineError::ValidationFailed(
                                "an AUTO rate that rounds to zero or below is invalid".into(),
                            ));
                        }
                        Some(Rate::parse(&rounded.to_string()).map_err(|e| {
                            RateEngineError::ValidationFailed(format!("invalid rate: {e}"))
                        })?)
                    } else {
                        None
                    }
                }
                _ => None,
            },
        };

        let sell = match &line.sell {
            LineSide::Manual(rate) => Some(*rate),
            LineSide::Auto => match (policy, reference) {
                (Some(pol), Some(ref_line)) => {
                    if let Some(ref_sell) = ref_line.sell {
                        let base = ref_sell.value() * units.unit_size().value()
                            / ref_line.unit_size.value();
                        let adjust =
                            Decimal::ONE + pol.sell_adjust_pct().value() / Decimal::from(100);
                        let raw = base * adjust;
                        let rounded = raw.round_dp_with_strategy(
                            8,
                            rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                        );
                        if rounded <= Decimal::ZERO {
                            return Err(RateEngineError::ValidationFailed(
                                "an AUTO rate that rounds to zero or below is invalid".into(),
                            ));
                        }
                        Some(Rate::parse(&rounded.to_string()).map_err(|e| {
                            RateEngineError::ValidationFailed(format!("invalid rate: {e}"))
                        })?)
                    } else {
                        None
                    }
                }
                _ => None,
            },
        };

        if let (Some(b), Some(s)) = (buy, sell) {
            if b.value() > s.value() {
                return Err(RateEngineError::ValidationFailed(
                    "buy rate must be less than or equal to sell rate".into(),
                ));
            }
        }

        Ok(ProposedLine { buy, sell })
    }
}
