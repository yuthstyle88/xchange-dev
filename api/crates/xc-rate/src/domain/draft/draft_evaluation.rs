//! Draft evaluation logic: checks guard boundaries and references.

use std::collections::HashMap;

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, Side};

use crate::domain::draft::rate_draft::{DraftLine, LineSide};
use crate::domain::draft::rate_proposal::ProposedLine;
use crate::domain::draft::tier_proposal::TierProposal;
use crate::domain::formula::price_formula::PriceFormula;
use crate::domain::policy::policy_tiers::PolicyTiers;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::reference::reference_batch::ReferenceLine;

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    out_of_bound: Vec<(CurrencyCode, Side)>,
}

impl Evaluation {
    pub fn new(out_of_bound: Vec<(CurrencyCode, Side)>) -> Self {
        Self { out_of_bound }
    }

    pub fn is_in_bound(&self) -> bool {
        self.out_of_bound.is_empty()
    }

    pub fn out_of_bound(&self) -> Vec<(CurrencyCode, Side)> {
        self.out_of_bound.clone()
    }
}

pub struct DraftEvaluation;

impl DraftEvaluation {
    /// Evaluates draft lines with per-currency resolved policy value tiers (`tiers[i]` for `lines[i]`) against policy guards.
    /// As `of_tiered`, with line `i`'s tiers re-proposed through `formulas[i]` (`TierProposal::of_with`) instead of
    /// the default, so the guard check sees the prices the draft will publish. `formulas.len() == lines.len()`.
    /// `of_tiered` becomes `of_tiered_with(.., &[PriceFormula::default(); n], units)`.
    pub fn of_tiered_with(
        lines: &[DraftLine],
        proposals: &[ProposedLine],
        references: &[ReferenceLine],
        policy: Option<&RatePolicy>,
        tiers: &[Option<PolicyTiers>],
        formulas: &[PriceFormula],
        units: &HashMap<CurrencyCode, CurrencyUnits>,
    ) -> Evaluation {
        let mut out_of_bound = Vec::new();
        if policy.is_none() {
            for line in lines {
                if !out_of_bound.contains(&(line.currency, Side::Buy)) {
                    out_of_bound.push((line.currency, Side::Buy));
                }
                if !out_of_bound.contains(&(line.currency, Side::Sell)) {
                    out_of_bound.push((line.currency, Side::Sell));
                }
            }
            return Evaluation::new(out_of_bound);
        }
        let default_formula = PriceFormula::default();

        for (i, line) in lines.iter().enumerate() {
            let Some(line_units) = units.get(&line.currency) else {
                if !out_of_bound.contains(&(line.currency, Side::Buy)) {
                    out_of_bound.push((line.currency, Side::Buy));
                }
                if !out_of_bound.contains(&(line.currency, Side::Sell)) {
                    out_of_bound.push((line.currency, Side::Sell));
                }
                continue;
            };
            if line.tiers.is_some() {
                continue;
            }
            let unit_size = line_units.unit_size().value();
            let proposal = match proposals.get(i) {
                Some(p) => p,
                None => continue,
            };

            let ref_line = references.iter().find(|r| r.currency == line.currency);

            // Untiered Buy side check
            if let Some(prop_buy) = proposal.buy {
                let is_out = match policy {
                    None => true,
                    Some(pol) => match ref_line.and_then(|r| r.buy.map(|b| (b, r.unit_size))) {
                        None => {
                            // If no reference buy, manual side is out-of-bound
                            matches!(line.buy, LineSide::Manual(_))
                        }
                        Some((ref_buy, ref_unit)) => {
                            match pol.guard_for(line.currency.as_str(), Side::Buy) {
                                None => true,
                                Some(guard) => {
                                    let norm_ref = ref_buy.value() * unit_size / ref_unit.value();
                                    if norm_ref.is_zero() {
                                        true
                                    } else {
                                        let distance = (prop_buy.value() - norm_ref) / norm_ref
                                            * Decimal::from(100);
                                        distance < guard.min_pct || distance > guard.max_pct
                                    }
                                }
                            }
                        }
                    },
                };
                if is_out && !out_of_bound.contains(&(line.currency, Side::Buy)) {
                    out_of_bound.push((line.currency, Side::Buy));
                }
            }

            // Untiered Sell side check
            if let Some(prop_sell) = proposal.sell {
                let is_out = match policy {
                    None => true,
                    Some(pol) => match ref_line.and_then(|r| r.sell.map(|s| (s, r.unit_size))) {
                        None => {
                            // If no reference sell, manual side is out-of-bound
                            matches!(line.sell, LineSide::Manual(_))
                        }
                        Some((ref_sell, ref_unit)) => {
                            match pol.guard_for(line.currency.as_str(), Side::Sell) {
                                None => true,
                                Some(guard) => {
                                    let norm_ref = ref_sell.value() * unit_size / ref_unit.value();
                                    if norm_ref.is_zero() {
                                        true
                                    } else {
                                        let distance = (prop_sell.value() - norm_ref) / norm_ref
                                            * Decimal::from(100);
                                        distance < guard.min_pct || distance > guard.max_pct
                                    }
                                }
                            }
                        }
                    },
                };
                if is_out && !out_of_bound.contains(&(line.currency, Side::Sell)) {
                    out_of_bound.push((line.currency, Side::Sell));
                }
            }

            // Tiered check (when value tiers are active for this line)
            if let Some(Some(pt)) = tiers.get(i) {
                if let Some(pol) = policy {
                    let formula = formulas.get(i).unwrap_or(&default_formula);

                    match TierProposal::of_with(
                        proposal,
                        line,
                        ref_line,
                        Some(pol),
                        pt,
                        line_units,
                        formula,
                    ) {
                        Ok(tier_lines) => {
                            for tl in tier_lines {
                                // Tier Buy check
                                if let Some(tier_buy) = tl.buy {
                                    if !out_of_bound.contains(&(line.currency, Side::Buy)) {
                                        let is_out = match ref_line
                                            .and_then(|r| r.buy.map(|b| (b, r.unit_size)))
                                        {
                                            None => matches!(line.buy, LineSide::Manual(_)),
                                            Some((ref_buy, ref_unit)) => {
                                                match pol
                                                    .guard_for(line.currency.as_str(), Side::Buy)
                                                {
                                                    None => true,
                                                    Some(guard) => {
                                                        let norm_ref = ref_buy.value() * unit_size
                                                            / ref_unit.value();
                                                        if norm_ref.is_zero() {
                                                            true
                                                        } else {
                                                            let distance = (tier_buy.value()
                                                                - norm_ref)
                                                                / norm_ref
                                                                * Decimal::from(100);
                                                            distance < guard.min_pct
                                                                || distance > guard.max_pct
                                                        }
                                                    }
                                                }
                                            }
                                        };
                                        if is_out {
                                            out_of_bound.push((line.currency, Side::Buy));
                                        }
                                    }
                                }

                                // Tier Sell check
                                if let Some(tier_sell) = tl.sell {
                                    if !out_of_bound.contains(&(line.currency, Side::Sell)) {
                                        let is_out = match ref_line
                                            .and_then(|r| r.sell.map(|s| (s, r.unit_size)))
                                        {
                                            None => matches!(line.sell, LineSide::Manual(_)),
                                            Some((ref_sell, ref_unit)) => {
                                                match pol
                                                    .guard_for(line.currency.as_str(), Side::Sell)
                                                {
                                                    None => true,
                                                    Some(guard) => {
                                                        let norm_ref = ref_sell.value() * unit_size
                                                            / ref_unit.value();
                                                        if norm_ref.is_zero() {
                                                            true
                                                        } else {
                                                            let distance = (tier_sell.value()
                                                                - norm_ref)
                                                                / norm_ref
                                                                * Decimal::from(100);
                                                            distance < guard.min_pct
                                                                || distance > guard.max_pct
                                                        }
                                                    }
                                                }
                                            }
                                        };
                                        if is_out {
                                            out_of_bound.push((line.currency, Side::Sell));
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            if !out_of_bound.contains(&(line.currency, Side::Buy)) {
                                out_of_bound.push((line.currency, Side::Buy));
                            }
                            if !out_of_bound.contains(&(line.currency, Side::Sell)) {
                                out_of_bound.push((line.currency, Side::Sell));
                            }
                        }
                    }
                }
            }
        }

        Evaluation::new(out_of_bound)
    }

    pub fn of_tiered(
        lines: &[DraftLine],
        proposals: &[ProposedLine],
        references: &[ReferenceLine],
        policy: Option<&RatePolicy>,
        tiers: &[Option<PolicyTiers>],
        units: &HashMap<CurrencyCode, CurrencyUnits>,
    ) -> Evaluation {
        Self::of_tiered_with(
            lines,
            proposals,
            references,
            policy,
            tiers,
            &vec![PriceFormula::default(); lines.len()],
            units,
        )
    }

    pub fn of(
        lines: &[DraftLine],
        proposals: &[ProposedLine],
        references: &[ReferenceLine],
        policy: Option<&RatePolicy>,
        units: &HashMap<CurrencyCode, CurrencyUnits>,
    ) -> Evaluation {
        let empty_tiers: Vec<Option<PolicyTiers>> = vec![None; lines.len()];
        Self::of_tiered(lines, proposals, references, policy, &empty_tiers, units)
    }
}
