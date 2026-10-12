//! Sharp-move detection for rate publications (SM-1 #1490, rates.md §12.8, §12.10).

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent, Side};

use crate::domain::currency::denomination_groups::GroupCode;

/// One comparable rate per (currency, group) and side, already reduced to the base for tiered lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparablePrice {
    pub currency: CurrencyCode,
    pub group: GroupCode,
    pub side: Side,
    pub rate: Option<Rate>,
}

impl ComparablePrice {
    pub fn new(currency: CurrencyCode, group: GroupCode, side: Side, rate: Rate) -> Self {
        Self {
            currency,
            group,
            side,
            rate: Some(rate),
        }
    }

    pub fn none(currency: CurrencyCode, group: GroupCode, side: Side) -> Self {
        Self {
            currency,
            group,
            side,
            rate: None,
        }
    }
}

/// A detected sharp move for a currency, denomination group, and side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub currency: CurrencyCode,
    pub group: GroupCode,
    pub side: Side,
    pub old: Rate,
    pub new: Rate,
    pub move_pct: Decimal,
}

use std::collections::HashMap;

pub struct SharpMove;

impl SharpMove {
    /// Detects sharp price movements between old and new comparable prices exceeding `threshold`.
    ///
    /// Pure function:
    /// - per (currency, denomination_group) and side: move_pct = |new − old| ÷ old × 100
    /// - a move only when move_pct > threshold: strictly greater, so exactly at the threshold is none
    /// - threshold = None ⇒ no moves at all
    /// - a group present in only one of old / new, or a side that is None on either, gives no move
    /// - old empty (no previous publication) ⇒ none
    pub fn between(
        old: &[ComparablePrice],
        new: &[ComparablePrice],
        threshold: Option<RatePercent>,
    ) -> Vec<Move> {
        let Some(threshold) = threshold else {
            return Vec::new();
        };
        if old.is_empty() || new.is_empty() {
            return Vec::new();
        }

        let old_map: HashMap<(&CurrencyCode, &GroupCode, Side), Option<Rate>> = old
            .iter()
            .map(|cp| ((&cp.currency, &cp.group, cp.side), cp.rate))
            .collect();

        let mut moves = Vec::new();
        for new_cp in new {
            let Some(new_rate) = new_cp.rate else {
                continue;
            };
            let Some(&Some(old_rate)) =
                old_map.get(&(&new_cp.currency, &new_cp.group, new_cp.side))
            else {
                continue;
            };

            let old_val = old_rate.value();
            let new_val = new_rate.value();
            let diff = (new_val - old_val).abs();
            let move_pct = (diff * Decimal::from(100) / old_val).normalize();

            if move_pct > threshold.value() {
                moves.push(Move {
                    currency: new_cp.currency,
                    group: new_cp.group.clone(),
                    side: new_cp.side,
                    old: old_rate,
                    new: new_rate,
                    move_pct,
                });
            }
        }

        moves
    }
}
