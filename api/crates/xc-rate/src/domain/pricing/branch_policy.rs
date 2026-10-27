//! How a branch follows MAIN (pricing-tiers.md §15.4; `H:` §7; V9:engine `resolvePolicy`, `adjustTiers`).
//!
//! SKELETON (XC-V9-PRICING): `BranchPricingPolicy::{resolve, adjust, check_bp}` are PV-a2's.
//! Types and signatures stay.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::PgConnection;

use crate::domain::pricing::market_quote::MarketQuote;
use crate::domain::pricing::pricing_suggestion::SuggestionConfig;
use crate::domain::pricing::tier_check::TierError;
use crate::domain::pricing::tier_set::{StoreFuture, TierRate};

/// One `pricing_branch_policy` row. `currency` is `"*"` (the branch default) or a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPolicyRow {
    pub currency: String,
    pub buy_bp: Option<i32>,
    pub sell_bp: Option<i32>,
    pub own: bool,
    pub follow: bool,
}

/// Where a resolved bp pair comes from (§15.8a `branchPolicy.from`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyFrom {
    Currency,
    Branch,
    Default,
}

impl PolicyFrom {
    /// `"currency"` · `"*"` · `"default"`.
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyFrom::Currency => "currency",
            PolicyFrom::Branch => "*",
            PolicyFrom::Default => "default",
        }
    }
}

/// The policy for one (branch, currency).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPolicy {
    pub buy_bp: i32,
    pub sell_bp: i32,
    /// The branch prices this currency itself: MAIN's push and FOLLOW never change it.
    pub own: bool,
    /// Explicit FOLLOW enrolment; only a currency row can set it (a branch's default never enrols it, `H:` §7).
    pub follow: bool,
    /// `Currency` when the currency row sets either bp, else `Branch` when the `'*'` row sets either, else `Default`.
    pub from: PolicyFrom,
}

/// One branch's rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchPricingPolicy {
    pub rows: Vec<BranchPolicyRow>,
}

impl BranchPricingPolicy {
    /// The bp limit either side (`H:` §7; the table's CHECK).
    pub const MAX_BP: i32 = 300;

    /// Each of `buyBp`/`sellBp`: the currency row's if not NULL, else the `'*'` row's, else 0. `own` and `follow` are
    /// the currency row's only.
    pub fn resolve(&self, currency: &str) -> ResolvedPolicy {
        let _ = (currency,);
        todo!("PV-a2: BranchPricingPolicy::resolve")
    }

    /// `buy[i] = floor_d(mainBuy[i] × (1 + buyBp/10000))`, `sell[i] = ceil_d(mainSell[i] × (1 + sellBp/10000))`, where
    /// `d` is the decimals of the base tiers (the largest scale among the 8 values), at most 5. Then monotonic:
    /// `buy[i] = max(buy[i], buy[i−1])`, `sell[i] = min(sell[i], sell[i−1])`. Then `TierCheck::validate` against
    /// `anchor` (per one unit; `None` skips the anchor check). `Err` holds every `TierError` found.
    pub fn adjust(
        base: &[TierRate; 4],
        policy: &ResolvedPolicy,
        unit_size: Decimal,
        anchor: Option<&MarketQuote>,
        config: &SuggestionConfig,
    ) -> Result<[TierRate; 4], Vec<TierError>> {
        let _ = (base, policy, unit_size, anchor, config);
        todo!("PV-a2: BranchPricingPolicy::adjust")
    }

    /// `Err` with the reason when `bp` is outside −300..=300 (`validationFailed`).
    pub fn check_bp(bp: i32) -> Result<i32, String> {
        let _ = (bp,);
        todo!("PV-a2: BranchPricingPolicy::check_bp")
    }
}

/// What `pricingBranchPolicySave` changes; `None` leaves a column as it is (a new row starts NULL / false).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BranchPolicyPatch {
    pub buy_bp: Option<i32>,
    pub sell_bp: Option<i32>,
    pub own: Option<bool>,
    pub follow: Option<bool>,
}

/// `pricing_branch_policy` (§15.2a). The Pg impl is `infrastructure::pricing::branch_policy_store`.
pub trait BranchPolicyStore: Send + Sync {
    /// Every row of the branch (`'*'` and currency rows).
    fn for_branch_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
    ) -> StoreFuture<'a, BranchPricingPolicy>;

    /// Upserts (branch, scope) with the patch applied. `scope` is `"*"` or a code.
    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        scope: &'a str,
        patch: BranchPolicyPatch,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, ()>;
}
