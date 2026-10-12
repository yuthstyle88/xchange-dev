//! Tier proposal logic: 4 value tiers per rate line (rates.md §12.3, §12.10).

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyUnits, Rate, Side};

use crate::domain::draft::rate_draft::{DraftLine, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::draft::rate_proposal::ProposedLine;
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
    /// Computes the four tier lines for a draft line using additive margin formula.
    pub fn of(
        proposed: &ProposedLine,
        line: &DraftLine,
        reference: Option<&ReferenceLine>,
        policy: Option<&RatePolicy>,
        tiers: &PolicyTiers,
        units: &CurrencyUnits,
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

            let buy = match &line.buy {
                LineSide::Manual(rate) => Some(*rate),
                LineSide::Auto => match (policy, reference) {
                    (Some(pol), Some(ref_line)) => {
                        if let Some(ref_buy) = ref_line.buy {
                            let base = ref_buy.value() * units.unit_size().value()
                                / ref_line.unit_size.value();
                            let margin = tiers.margin(tier_no, Side::Buy);
                            let effective_adjust = pol.buy_adjust_pct().value() + margin.value();
                            let adjust = Decimal::ONE + effective_adjust / Decimal::from(100);
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
                            let margin = tiers.margin(tier_no, Side::Sell);
                            let effective_adjust = pol.sell_adjust_pct().value() + margin.value();
                            let adjust = Decimal::ONE + effective_adjust / Decimal::from(100);
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
}
