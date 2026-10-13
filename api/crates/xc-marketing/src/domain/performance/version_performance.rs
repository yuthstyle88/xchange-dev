//! One M04 row (D-15): a campaign version's figures over a scope and a period.

use uuid::Uuid;
use xc_kernel::money::{Money, MoneyDelta};

use super::unavailable::Unavailable;

/// Counted over the version's redemptions that are REDEEMED, in scope and in the period; REVERSED ones add nothing
/// (ruling 2 on #1406). "Redeeming exchanges" are the distinct transactions of those redemptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPerformance {
    pub campaign_id: Uuid,
    pub campaign_name: String,
    pub version_id: Uuid,
    pub version: i32,
    /// The number of counted redemptions.
    pub redemptions: i64,
    /// Σ `amount_thb` of the counted PRICE-class redemptions (already inside the margin).
    pub price_cost: Money,
    /// Σ `amount_thb` of the counted REWARD-class redemptions.
    pub reward_expense: Money,
    /// Σ `transaction_lines.thb_amount` of the redeeming exchanges, each exchange once.
    pub volume_thb: Money,
    /// Σ `transaction_lines.realized_margin_thb` of the redeeming exchanges; `AwaitsLedger` if any of their lines has
    /// none.
    pub realized_margin: Result<MoneyDelta, Unavailable>,
    /// [`super::contribution::attributed_contribution`] of the above and the commission of the redeeming exchanges.
    pub contribution: Result<MoneyDelta, Unavailable>,
}
