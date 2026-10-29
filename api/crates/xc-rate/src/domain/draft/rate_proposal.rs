//! Rate proposal logic: draft line + reference + policy -> proposed rates.

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyUnits, Rate, Side};

use crate::domain::draft::rate_draft::{DraftLine, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::formula::error::FormulaError;
use crate::domain::formula::input::FormulaInputs;
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

        let inputs = match (policy, reference) {
            (Some(pol), Some(ref_line)) => {
                let scale =
                    |r: Rate| r.value() * units.unit_size().value() / ref_line.unit_size.value();
                Some(FormulaInputs {
                    ref_buy: ref_line.buy.map(scale),
                    ref_sell: ref_line.sell.map(scale),
                    buy_pct: pol.buy_adjust_pct().value(),
                    sell_pct: pol.sell_adjust_pct().value(),
                    tier_buy_pct: Decimal::ZERO,
                    tier_sell_pct: Decimal::ZERO,
                })
            }
            _ => None,
        };

        let buy = Self::price_side(&line.buy, Side::Buy, inputs.as_ref(), formula)?;
        let sell = Self::price_side(&line.sell, Side::Sell, inputs.as_ref(), formula)?;

        if let (Some(b), Some(s)) = (buy, sell) {
            if b.value() > s.value() {
                return Err(RateEngineError::ValidationFailed(
                    "buy rate must be less than or equal to sell rate".into(),
                ));
            }
        }

        Ok(ProposedLine { buy, sell })
    }

    pub fn of(
        line: &DraftLine,
        reference: Option<&ReferenceLine>,
        policy: Option<&RatePolicy>,
        units: &CurrencyUnits,
    ) -> Result<ProposedLine, RateEngineError> {
        Self::of_with(line, reference, policy, units, &PriceFormula::default())
    }

    fn price_side(
        line_side: &LineSide,
        side: Side,
        inputs: Option<&FormulaInputs>,
        formula: &PriceFormula,
    ) -> Result<Option<Rate>, RateEngineError> {
        match line_side {
            LineSide::Manual(rate) => Ok(Some(*rate)),
            LineSide::Auto => match inputs {
                Some(inp) => match formula.side(side, inp) {
                    Ok(rate) => Ok(Some(rate)),
                    Err(FormulaError::MissingInput(_)) => Ok(None),
                    Err(e) => Err(RateEngineError::ValidationFailed(e.to_string())),
                },
                None => Ok(None),
            },
        }
    }
}
