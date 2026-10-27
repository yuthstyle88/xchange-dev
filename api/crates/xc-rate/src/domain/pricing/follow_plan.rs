//! FOLLOW at MAIN: plan one currency's tiers from the market, or say why not (pricing-tiers.md §15.5; `H:` §6;
//! V9:automatic `planAutomaticPricing`). Pure: no I/O, the caller passes `now`.
//!
//! SKELETON (XC-V9-PRICING): `SourceFreshness::is_current`, `FollowReason::as_str`, `FollowPlan::plan` and
//! `FollowResultStatus::{as_str, parse}` are PV-f's. Types and signatures stay.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;
use xc_kernel::money::CurrencyCode;

use crate::domain::pricing::pricing_suggestion::SuggestionInput;
use crate::domain::pricing::tier_set::{MarketAnchor, StoreFuture, TierRate};

/// The anchor source's last fetch (`market_feed_status` + the newest snapshot of that source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFreshness {
    /// The newest good snapshot's `fetched_at`; `None` when there is none.
    pub fetched_at: Option<DateTime<Utc>>,
    /// `market_feed_status.last_attempt_at`; `None` when the source was never tried.
    pub last_attempt_at: Option<DateTime<Utc>>,
    /// `market_feed_status.last_error`.
    pub error: Option<String>,
}

impl SourceFreshness {
    /// `H:` §6: the last fetch succeeded — no error, `fetched_at == last_attempt_at`, not in the future, and at most
    /// `MarketStaleness::STALE_AFTER_MINUTES` (30) old at `now`. Shared with `FollowVisibility` (§15.7 `STALE`).
    pub fn is_current(&self, now: DateTime<Utc>) -> bool {
        let _ = (now,);
        todo!("PV-f: SourceFreshness::is_current")
    }
}

/// Why a FOLLOW plan is blocked (`pricing_follow_results.reasons`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FollowReason {
    NoAnchor,
    StaleSource,
    Precision,
    BelowCost,
    LargeMove,
    /// `TierCheck` refused the tiers, or they are not at most 4 decimals.
    InvalidTiers,
}

impl FollowReason {
    /// `"NO_ANCHOR"` · `"STALE_SOURCE"` · `"PRECISION"` · `"BELOW_COST"` · `"LARGE_MOVE"` · `"INVALID_TIERS"`.
    pub fn as_str(self) -> &'static str {
        todo!("PV-f: FollowReason::as_str")
    }
}

/// What `FollowPlan::plan` reads for one currency.
#[derive(Debug, Clone)]
pub struct FollowInput {
    /// The suggestion's input, with the currency's settings resolved (`better`, `min_spread_pct`) and
    /// `published_tier1_*` set to MAIN's currently published tier 1.
    pub suggestion: SuggestionInput,
    /// The anchor source's last fetch (TT's, or SR's when the anchor is SR).
    pub source: SourceFreshness,
    /// The anchor's `reference_batches.id`.
    pub anchor_batch_id: Option<Uuid>,
    /// Tier 1 of MAIN's latest tier set (any source), at the unit size.
    pub last_tier1: Option<TierRate>,
    /// Tier 1 of MAIN's current publication, at the unit size.
    pub published_tier1: Option<TierRate>,
}

/// The plan for one currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowOutcome {
    Candidate([TierRate; 4], MarketAnchor),
    Blocked(Vec<FollowReason>),
}

pub struct FollowPlan;

impl FollowPlan {
    /// `PricingSuggestion::of(&input.suggestion)`, then blocked (every reason found, in `FollowReason` order) when:
    /// - `NoAnchor`: no anchor;
    /// - `StaleSource`: `!input.source.is_current(now)`;
    /// - `Precision` / `BelowCost` / `LargeMove`: that flag is in the suggestion's flags;
    /// - `LargeMove`: tier 1 moves more than `config.large_move_pct` against `last_tier1` **or** `published_tier1`,
    ///   on the buy **or** the sell side (`H:` §6: both sides, against both);
    /// - `InvalidTiers`: `TierCheck::validate` refuses the tiers, or a price has more than 4 decimals.
    ///
    /// Otherwise `Candidate` with the suggested tiers and the anchor.
    pub fn plan(input: &FollowInput, now: DateTime<Utc>) -> FollowOutcome {
        let _ = (input, now);
        todo!("PV-f: FollowPlan::plan")
    }
}

/// `pricing_follow_results.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FollowResultStatus {
    /// Planned but not published (yet).
    Candidate,
    Blocked,
    /// Published as `tier_set_id`.
    Published,
}

impl FollowResultStatus {
    /// `"CANDIDATE"` · `"BLOCKED"` · `"PUBLISHED"`.
    pub fn as_str(self) -> &'static str {
        todo!("PV-f: FollowResultStatus::as_str")
    }

    pub fn parse(text: &str) -> Option<Self> {
        let _ = (text,);
        todo!("PV-f: FollowResultStatus::parse")
    }
}

/// One `pricing_follow_results` row: the latest plan of a currency at MAIN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowResult {
    pub currency: CurrencyCode,
    pub status: FollowResultStatus,
    pub reasons: Vec<FollowReason>,
    pub anchor_batch_id: Option<Uuid>,
    pub tier_set_id: Option<Uuid>,
    pub planned_at: DateTime<Utc>,
}

/// `pricing_follow_results` (§15.2a). The Pg impl is `infrastructure::pricing::follow_result_store`.
pub trait FollowResultStore: Send + Sync {
    /// Upserts the currency's row.
    fn record_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        result: &'a FollowResult,
    ) -> StoreFuture<'a, ()>;

    fn latest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<FollowResult>>;

    /// `max(planned_at)` over every row, for `pricing_board`'s version (§15.9).
    fn newest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Option<DateTime<Utc>>>;
}
