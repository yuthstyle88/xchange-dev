//! A pricing board row's status (pricing-tiers.md §5.1, V.9 `statusOf` in `src/devices-public/pricing.js:84-94`).

use crate::domain::pricing::pricing_suggestion::{Suggestion, TierPrice};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowStatus {
    Current,
    Pending,
    Unpriced,
    Tiers,
    Review,
    Manual,
    Reference,
}

pub struct RowFacts<'a> {
    pub suggestion: &'a Suggestion,
    pub priceable: bool,
    pub published: Option<&'a [TierPrice]>,
}

impl RowStatus {
    pub fn of_row(facts: &RowFacts<'_>) -> RowStatus {
        if !facts.priceable {
            return Self::Reference;
        }
        let Some(tiers) = &facts.suggestion.tiers else {
            return Self::Manual;
        };
        if facts.suggestion.blocked() {
            return Self::Review;
        }
        let published = facts.published.unwrap_or(&[]);
        if (0..4).all(|i| {
            published
                .iter()
                .find(|p| p.tier_no == tiers[i].tier_no)
                .is_some_and(|p| p.buy == tiers[i].buy && p.sell == tiers[i].sell)
        }) {
            return Self::Current;
        }
        let Some(p1) = published.iter().find(|p| p.tier_no == 1) else {
            return Self::Unpriced;
        };
        if p1.buy.is_none() {
            return Self::Unpriced;
        }
        if p1.buy == tiers[0].buy && p1.sell == tiers[0].sell {
            Self::Tiers
        } else {
            Self::Pending
        }
    }

    /// Kept for PT-8's domain test; the pricing board must use [`Self::of_row`].
    pub fn of(suggestion: &Suggestion, _approved: bool, _enabled: bool) -> RowStatus {
        Self::of_row(&RowFacts {
            suggestion,
            priceable: true,
            published: None,
        })
    }
}
