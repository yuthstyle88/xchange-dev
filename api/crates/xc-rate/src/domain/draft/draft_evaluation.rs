//! Draft evaluation logic: checks guard boundaries and references.

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, Side};

use crate::domain::draft::rate_draft::{DraftLine, LineSide};
use crate::domain::draft::rate_proposal::ProposedLine;
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

fn currency_unit_size(code: CurrencyCode) -> Decimal {
    match code.as_str() {
        "JPY" => Decimal::from(100),
        "KRW" => Decimal::from(1000),
        _ => Decimal::ONE,
    }
}

impl DraftEvaluation {
    pub fn of(
        lines: &[DraftLine],
        proposals: &[ProposedLine],
        references: &[ReferenceLine],
        policy: Option<&RatePolicy>,
    ) -> Evaluation {
        let mut out_of_bound = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            let proposal = match proposals.get(i) {
                Some(p) => p,
                None => continue,
            };

            let ref_line = references.iter().find(|r| r.currency == line.currency);

            // Buy side check
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
                                    let norm_ref = ref_buy.value()
                                        * currency_unit_size(line.currency)
                                        / ref_unit.value();
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
                if is_out {
                    out_of_bound.push((line.currency, Side::Buy));
                }
            }

            // Sell side check
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
                                    let norm_ref = ref_sell.value()
                                        * currency_unit_size(line.currency)
                                        / ref_unit.value();
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
                if is_out {
                    out_of_bound.push((line.currency, Side::Sell));
                }
            }
        }

        Evaluation::new(out_of_bound)
    }
}
