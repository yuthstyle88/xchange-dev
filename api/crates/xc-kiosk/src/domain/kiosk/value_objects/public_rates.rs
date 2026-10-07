//! What the kiosk screen is told before a quote.

use xc_kernel::ids::BranchId;
use xc_kernel::payment::PaymentRules;

use crate::domain::kiosk::value_objects::counter_view::CounterRate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRates {
    pub branch_id: BranchId,
    pub branch_name: String,
    pub open: bool,
    /// Only the currencies the kiosk offers and the counter has a rate for.
    pub rates: Vec<CounterRate>,
    /// The branch's payment rules; the route prints them public, without `account` (K-HTTP).
    pub payment: PaymentRules,
}
