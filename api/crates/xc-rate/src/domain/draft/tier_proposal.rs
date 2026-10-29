//! Tier proposal logic: 4 value tiers per rate line (rates.md §12.3, §12.10).

use xc_kernel::money::{CurrencyUnits, Rate, Side};

use crate::domain::draft::rate_draft::{DraftLine, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::draft::rate_proposal::ProposedLine;
use crate::domain::formula::error::FormulaError;
use crate::domain::formula::input::FormulaInputs;
use crate::domain::formula::price_formula::PriceFormula;
use crate::domain::policy::policy_tiers::PolicyTiers;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::publication::value_tier::{TierBand, TierCode};
use crate::domain::reference::reference_batch::ReferenceLine;

/// Proposed rates for one tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierLine {
    pub code: TierCode,
    pub band: TierBand,
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
    pub base_buy: Option<Rate>,
    pub base_sell: Option<Rate>,
}

pub struct TierProposal;

impl TierProposal {
    /// As `of`, with each AUTO tier side priced by `formula`: `tier_buy_pct` / `tier_sell_pct` = the tier's margin
    /// (`PolicyTiers::margin`), the rest as `RateProposal::of_with`. With `PriceFormula::default()` the answer
    /// equals `of`'s (B1); `of` becomes `of_with(.., &PriceFormula::default())`.
    pub fn of_with(
        proposed: &ProposedLine,
        line: &DraftLine,
        reference: Option<&ReferenceLine>,
        policy: Option<&RatePolicy>,
        tiers: &PolicyTiers,
        units: &CurrencyUnits,
        formula: &PriceFormula,
    ) -> Result<Vec<TierLine>, RateEngineError> {
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

        let bands = tiers.bands();
        let mut tier_lines = Vec::with_capacity(4);

        for (idx, band) in bands.into_iter().enumerate() {
            let tier_no = (idx + 1) as u8;
            let inputs = match (policy, reference) {
                (Some(pol), Some(ref_line)) => {
                    let scale = units.unit_size().value() / ref_line.unit_size.value();
                    Some(FormulaInputs {
                        ref_buy: ref_line.buy.map(|r| r.value() * scale),
                        ref_sell: ref_line.sell.map(|r| r.value() * scale),
                        buy_pct: pol.buy_adjust_pct().value(),
                        sell_pct: pol.sell_adjust_pct().value(),
                        tier_buy_pct: tiers.margin(tier_no, Side::Buy).value(),
                        tier_sell_pct: tiers.margin(tier_no, Side::Sell).value(),
                    })
                }
                _ => None,
            };

            let price_side = |line_side: &LineSide, side: Side| match line_side {
                LineSide::Manual(rate) => Ok(Some(*rate)),
                LineSide::Auto => match &inputs {
                    Some(inp) => match formula.side(side, inp) {
                        Ok(rate) => Ok(Some(rate)),
                        Err(FormulaError::MissingInput(_)) => Ok(None),
                        Err(e) => Err(RateEngineError::ValidationFailed(e.to_string())),
                    },
                    None => Ok(None),
                },
            };

            let buy = price_side(&line.buy, Side::Buy)?;
            let sell = price_side(&line.sell, Side::Sell)?;

            if let (Some(b), Some(s)) = (buy, sell) {
                if b.value() > s.value() {
                    return Err(RateEngineError::ValidationFailed(format!(
                        "tier {} buy rate ({}) must be less than or equal to sell rate ({})",
                        tier_no,
                        b.value(),
                        s.value()
                    )));
                }
            }

            tier_lines.push(TierLine {
                code: band.code.clone(),
                band,
                buy,
                sell,
                base_buy: proposed.buy,
                base_sell: proposed.sell,
            });
        }

        Ok(tier_lines)
    }

    /// Computes the four tier lines for a draft line using additive margin formula.
    pub fn of(
        proposed: &ProposedLine,
        line: &DraftLine,
        reference: Option<&ReferenceLine>,
        policy: Option<&RatePolicy>,
        tiers: &PolicyTiers,
        units: &CurrencyUnits,
    ) -> Result<Vec<TierLine>, RateEngineError> {
        Self::of_with(
            proposed,
            line,
            reference,
            policy,
            tiers,
            units,
            &PriceFormula::default(),
        )
    }
}
