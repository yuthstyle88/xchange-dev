//! The weighted average of one position (Q10; design §3.2). Pure, no database. Scaffold by the
//! ticket author: the bodies are `todo!()`.

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

/// A position: `quantity` units that cost `cost_thb` in total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightedAverage {
    pub quantity: Decimal,
    pub cost_thb: Decimal,
}

impl WeightedAverage {
    /// Nothing held.
    pub const EMPTY: WeightedAverage = WeightedAverage {
        quantity: Decimal::ZERO,
        cost_thb: Decimal::ZERO,
    };

    /// Adds `quantity` units that cost `cost_thb`.
    pub fn acquire(self, quantity: Decimal, cost_thb: Decimal) -> Self {
        Self {
            quantity: self.quantity + quantity,
            cost_thb: self.cost_thb + cost_thb,
        }
    }

    /// Takes `quantity` units out at the average. Answers their cost, rounded half away from zero to
    /// 2 places, and the position left. Taking every unit takes the whole remaining `cost_thb`.
    pub fn dispose(self, quantity: Decimal) -> (Decimal, Self) {
        if quantity == self.quantity {
            (
                self.cost_thb,
                Self {
                    quantity: Decimal::ZERO,
                    cost_thb: Decimal::ZERO,
                },
            )
        } else {
            let cost = (quantity * self.cost_thb / self.quantity)
                .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
            (
                cost,
                Self {
                    quantity: self.quantity - quantity,
                    cost_thb: self.cost_thb - cost,
                },
            )
        }
    }
}
