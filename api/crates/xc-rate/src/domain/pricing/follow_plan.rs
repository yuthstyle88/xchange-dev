//! FOLLOW at MAIN: plan one currency's tiers from the market, or say why not (pricing-tiers.md §15.5; `H:` §6;
//! V9:automatic `planAutomaticPricing`). Pure: no I/O, the caller passes `now`.
//!
//! SKELETON (XC-V9-PRICING): `SourceFreshness::is_current` and `FollowPlan::plan` are PV-f1's; the wire strings are
//! the contract and are written here. Types and signatures stay.

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use sqlx::PgConnection;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Rate};

use crate::domain::pricing::market_quote::MarketQuote;
use crate::domain::pricing::market_snapshot::MarketStaleness;
use crate::domain::pricing::pricing_flag::PricingFlag;
use crate::domain::pricing::pricing_suggestion::{PricingSuggestion, SuggestionInput};
use crate::domain::pricing::tier_check::TierCheck;
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
    /// Builds the freshness from v3's market feed (pricing-tiers.md §15.5; `H:` §6 "the last fetch succeeded"). v3's
    /// `ReferenceBatchWriter` writes a new batch only when the answer changed, and `MarketRefresh::tick` records the
    /// attempt at its own `now`, so the newest batch's `fetched_at` never equals `last_attempt_at` even when the last
    /// fetch succeeded. Reading: when the last attempt has **no error** and a batch exists, that attempt is the last
    /// successful fetch, so `fetched_at := last_attempt_at`; otherwise `fetched_at` is the newest batch's (and an error
    /// keeps the source not current). Both FOLLOW readers (`MarketPricingLoop`, the public projection) build it here.
    ///
    /// SKELETON (XC-V9-PRICING): PV-g's.
    pub fn of_feed(
        newest_batch_fetched_at: Option<DateTime<Utc>>,
        last_attempt_at: Option<DateTime<Utc>>,
        error: Option<String>,
    ) -> Self {
        let fetched_at = if error.is_none() && newest_batch_fetched_at.is_some() {
            last_attempt_at.or(newest_batch_fetched_at)
        } else {
            newest_batch_fetched_at
        };
        Self {
            fetched_at,
            last_attempt_at,
            error,
        }
    }

    /// `H:` §6: the last fetch succeeded — no error, `fetched_at == last_attempt_at`, not in the future, and at most
    /// `MarketStaleness::STALE_AFTER_MINUTES` (30) old at `now`. Shared with `FollowVisibility` (§15.7 `STALE`).
    pub fn is_current(&self, now: DateTime<Utc>) -> bool {
        let (Some(fetched_at), Some(last_attempt_at)) = (self.fetched_at, self.last_attempt_at)
        else {
            return false;
        };
        self.error.is_none()
            && fetched_at == last_attempt_at
            && fetched_at <= now
            && now - fetched_at <= Duration::minutes(MarketStaleness::STALE_AFTER_MINUTES)
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
        match self {
            FollowReason::NoAnchor => "NO_ANCHOR",
            FollowReason::StaleSource => "STALE_SOURCE",
            FollowReason::Precision => "PRECISION",
            FollowReason::BelowCost => "BELOW_COST",
            FollowReason::LargeMove => "LARGE_MOVE",
            FollowReason::InvalidTiers => "INVALID_TIERS",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [
            FollowReason::NoAnchor,
            FollowReason::StaleSource,
            FollowReason::Precision,
            FollowReason::BelowCost,
            FollowReason::LargeMove,
            FollowReason::InvalidTiers,
        ]
        .into_iter()
        .find(|r| r.as_str() == text)
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
    /// - `NoAnchor`: no **market** anchor (`MarketQuote::anchor_for`). The suggestion also flags `NoAnchor` when the
    ///   draft path's BoT/START base is missing (`pricing_suggestion.rs:252-256`); FOLLOW never uses that base
    ///   (§15.5, gap 7), so that flag alone does not block;
    /// - `StaleSource`: `!input.source.is_current(now)`;
    /// - `Precision` / `BelowCost` / `LargeMove`: that flag is in the suggestion's flags;
    /// - `LargeMove`: tier 1 moves more than `config.large_move_pct` against `last_tier1` **or** `published_tier1`,
    ///   on the buy **or** the sell side (`H:` §6: both sides, against both);
    /// - `InvalidTiers`: `TierCheck::validate` refuses the tiers, or a price has more than 4 decimals.
    ///
    /// Otherwise `Candidate` with the suggested tiers and the anchor.
    pub fn plan(input: &FollowInput, now: DateTime<Utc>) -> FollowOutcome {
        let (anchor_quote, _) = MarketQuote::anchor_for(&input.suggestion.quotes);
        let suggestion = PricingSuggestion::of(&input.suggestion);
        let mut reasons = Vec::new();

        if anchor_quote.is_none() {
            reasons.push(FollowReason::NoAnchor);
        }
        if !input.source.is_current(now) {
            reasons.push(FollowReason::StaleSource);
        }
        if suggestion.flags.contains(&PricingFlag::Precision) {
            reasons.push(FollowReason::Precision);
        }
        if suggestion.flags.contains(&PricingFlag::BelowCost) {
            reasons.push(FollowReason::BelowCost);
        }

        let moved = |cur: Rate, prev: Rate| {
            prev.value() > Decimal::ZERO
                && ((cur.value() - prev.value()) / prev.value() * Decimal::from(100)).abs()
                    > input.suggestion.config.large_move_pct
        };
        let large_move = suggestion.flags.contains(&PricingFlag::LargeMove)
            || suggestion
                .tiers
                .as_ref()
                .and_then(|t| t[0].buy.zip(t[0].sell))
                .is_some_and(|(t1_buy, t1_sell)| {
                    [input.last_tier1, input.published_tier1]
                        .into_iter()
                        .flatten()
                        .any(|prev| moved(t1_buy, prev.buy) || moved(t1_sell, prev.sell))
                });
        if large_move {
            reasons.push(FollowReason::LargeMove);
        }

        if let Some(tiers) = &suggestion.tiers {
            let valid_decimals = tiers.iter().all(|t| {
                t.buy.is_some_and(|r| r.value().normalize().scale() <= 4)
                    && t.sell.is_some_and(|r| r.value().normalize().scale() <= 4)
            });
            if !TierCheck::validate(
                tiers,
                input.suggestion.unit_size,
                anchor_quote,
                &input.suggestion.config,
            )
            .is_empty()
                || !valid_decimals
            {
                reasons.push(FollowReason::InvalidTiers);
            }
        }

        if !reasons.is_empty() {
            return FollowOutcome::Blocked(reasons);
        }

        let Some(anchor) = anchor_quote else {
            return FollowOutcome::Blocked(vec![FollowReason::NoAnchor]);
        };
        let (Some(anchor_buy), Some(anchor_sell), Some(tiers)) =
            (anchor.buy, anchor.sell, suggestion.tiers)
        else {
            return FollowOutcome::Blocked(vec![FollowReason::NoAnchor]);
        };
        let tier_rates = std::array::from_fn(|i| TierRate {
            buy: tiers[i].buy.expect("validated"),
            sell: tiers[i].sell.expect("validated"),
        });
        FollowOutcome::Candidate(
            tier_rates,
            MarketAnchor {
                source: anchor.source,
                buy: anchor_buy,
                sell: anchor_sell,
                fetched_at: anchor.fetched_at,
                batch_id: input.anchor_batch_id,
            },
        )
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
        match self {
            FollowResultStatus::Candidate => "CANDIDATE",
            FollowResultStatus::Blocked => "BLOCKED",
            FollowResultStatus::Published => "PUBLISHED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "CANDIDATE" => Some(FollowResultStatus::Candidate),
            "BLOCKED" => Some(FollowResultStatus::Blocked),
            "PUBLISHED" => Some(FollowResultStatus::Published),
            _ => None,
        }
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
