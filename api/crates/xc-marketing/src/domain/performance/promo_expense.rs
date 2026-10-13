//! `PromoExpenseFacts` (design §6 `PerformanceReader ..|> PromoExpenseFacts`, §6.1; ruling 3 on #1406): the shape
//! mirrors executive's `CommissionSource` (`api/src/application/executive/ports.rs`).

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::money::Money;

use super::period::{PerformancePeriod, PerformanceScope};
use super::unavailable::Unavailable;

/// The period's promo/reward expense not already in the rate, for executive's `contribution_thb`.
pub trait PromoExpenseFacts: Send + Sync {
    /// Σ `amount_thb` of the scope's REWARD-class redemptions that are REDEEMED in the period.
    ///
    /// - PRICE-class redemptions never count: their cost is already inside the realized margin (D-15).
    /// - REVERSED redemptions never count: their budget was returned (owner M3; ruling 2).
    /// - No redemption ⇒ `Ok(Ok(0.00))`, a real zero; never `Unavailable` now that campaigns exist.
    fn reward_expense<'a>(
        &'a self,
        scope: &'a PerformanceScope,
        period: &'a PerformancePeriod,
    ) -> BoxFuture<'a, AppResult<Result<Money, Unavailable>>>;
}
