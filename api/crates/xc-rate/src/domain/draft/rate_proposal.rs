//! Rate proposal logic: draft line + reference + policy -> proposed rates.

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyUnits, Rate};

use crate::domain::draft::rate_draft::{DraftLine, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::formula::price_formula::PriceFormula;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::reference::reference_batch::ReferenceLine;

#[derive(Debug, Clone, PartialEq)]
pub struct ProposedLine {
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
}

pub struct RateProposal;

impl RateProposal {
    /// The AUTO sides priced by `formula` (pricing-formula.md §2, §6a): inputs `ref_buy` / `ref_sell` = the
    /// reference side at `units`' unit size (as today's `base`), `buy_pct` / `sell_pct` = the policy's,
    /// `tier_buy_pct` = `tier_sell_pct` = 0. A side whose formula names a missing reference side
    /// (`FormulaError::MissingInput`) is `None`, as today when the reference has no such side; any other
    /// `FormulaError` ⇒ `RateEngineError::ValidationFailed`. MANUAL sides, the reason rule and the buy ≤ sell check
    /// are unchanged. With `PriceFormula::default()` the answer equals `of`'s to the 8th place (B1); `of` becomes
    /// `of_with(.., &PriceFormula::default())`.
    pub fn of_with(
        line: &DraftLine,
        reference: Option<&ReferenceLine>,
        policy: Option<&RatePolicy>,
        units: &CurrencyUnits,
        formula: &PriceFormula,
    ) -> Result<ProposedLine, RateEngineError> {
        let _ = (line, reference, policy, units, formula);
        todo!("pricing-formula wave 2: RateProposal::of_with")
    }

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
