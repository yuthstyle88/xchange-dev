//! XC-V9-PRICING PV-11 / PV-12 / PV-13 (pricing-tiers.md §15.5; `H:` §6; V9:automatic `planAutomaticPricing`): when the
//! anchor source is current, and the FOLLOW plan at MAIN — a candidate with V.9's suggested tiers and the anchor, or
//! blocked with every reason found. `H:` is the owner's handoff `PRICING-DESIGN.md` and `V9:` its source excerpt (both
//! kept out of the repo). Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING
//! skeleton). Do not edit, delete or weaken; add tests in a file of your own. Pure: no database; `now` is passed in.

use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent};
use xc_rate::domain::pricing::follow_plan::{
    FollowInput, FollowOutcome, FollowPlan, FollowReason, SourceFreshness,
};
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_suggestion::{SuggestionConfig, SuggestionInput};
use xc_rate::domain::pricing::tier_set::{MarketAnchor, TierRate};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("rate")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 3, 0, 0).unwrap()
}

fn minutes_ago(m: i64) -> DateTime<Utc> {
    now() - Duration::minutes(m)
}

const BATCH: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_b001);

fn tier(buy: &str, sell: &str) -> TierRate {
    TierRate {
        buy: rate(buy),
        sell: rate(sell),
    }
}

/// PV-1's tiers (`H:` §4.4).
fn pv1() -> [TierRate; 4] {
    [
        tier("33.23", "33.87"),
        tier("33.24", "33.86"),
        tier("33.26", "33.84"),
        tier("33.29", "33.80"),
    ]
}

/// A source fetched `m` minutes ago whose last attempt was that fetch, with no error.
fn fetched(m: i64) -> SourceFreshness {
    SourceFreshness {
        fetched_at: Some(minutes_ago(m)),
        last_attempt_at: Some(minutes_ago(m)),
        error: None,
    }
}

fn quote(source: MarketSource, buy: &str, sell: &str, fetched_at: DateTime<Utc>) -> MarketQuote {
    MarketQuote {
        source,
        buy: Some(rate(buy)),
        sell: Some(rate(sell)),
        unit_size: Decimal::ONE,
        fetched_at,
        // The source's freshness is `FollowInput::source`'s to say; the quote's own flag stays off so each test below
        // isolates the rule it names.
        stale: false,
    }
}

/// FOLLOW USD at MAIN, built as `pricing_suggestion_test.rs`'s `usd()`: TT USD(L) 33.20 / 33.90 (d = 2) and Super
/// Rich 33.33 / 33.40, BoT base 33 / 34, V.9's defaults; TT fetched `m` minutes ago; nothing approved or published.
fn usd_fetched(m: i64) -> FollowInput {
    FollowInput {
        suggestion: SuggestionInput {
            currency: CurrencyCode::parse("USD").expect("currency"),
            quotes: vec![
                quote(MarketSource::Tt, "33.2", "33.9", minutes_ago(m)),
                quote(MarketSource::SuperRich, "33.33", "33.40", minutes_ago(m)),
            ],
            tt_decimals: Some(2),
            unit_size: Decimal::ONE,
            base_buy: Some(rate("33")),
            base_sell: Some(rate("34")),
            buy_adjust_pct: RatePercent::ZERO,
            sell_adjust_pct: RatePercent::ZERO,
            partner_cost: None,
            published_tier1_buy: None,
            published_tier1_sell: None,
            config: SuggestionConfig::v9_default(),
        },
        source: fetched(m),
        anchor_batch_id: Some(BATCH),
        last_tier1: None,
        published_tier1: None,
    }
}

/// Sets MAIN's current publication's tier 1, in both places `FollowInput` carries it.
fn published(mut input: FollowInput, buy: &str, sell: &str) -> FollowInput {
    input.suggestion.published_tier1_buy = Some(rate(buy));
    input.suggestion.published_tier1_sell = Some(rate(sell));
    input.published_tier1 = Some(tier(buy, sell));
    input
}

fn plan(input: &FollowInput) -> FollowOutcome {
    FollowPlan::plan(input, now())
}

fn reasons(outcome: FollowOutcome) -> Vec<FollowReason> {
    match outcome {
        FollowOutcome::Blocked(r) => r,
        other => panic!("expected Blocked, got {other:?}"),
    }
}

// ── SourceFreshness::is_current ────────────────────────────────────────────────────────────────────────────────

/// PV-11 (§15.5; `H:` §6 "อายุไม่เกิน 30 นาที"): fetched exactly 30 minutes ago is current.
#[test]
fn a_source_fetched_thirty_minutes_ago_is_current() {
    assert!(fetched(30).is_current(now()));
    assert!(fetched(0).is_current(now()));
}

/// PV-11 (§15.5; `H:` §6): fetched 31 minutes ago is not current.
#[test]
fn a_source_fetched_thirty_one_minutes_ago_is_not_current() {
    assert!(!fetched(31).is_current(now()));
}

/// PV-11 (§15.5; `H:` §6 "ไม่มี error"): the last fetch failed — not current, however recent the good snapshot.
#[test]
fn a_source_whose_last_fetch_failed_is_not_current() {
    let failed = SourceFreshness {
        error: Some("timeout".to_string()),
        ..fetched(1)
    };
    assert!(!failed.is_current(now()));
}

/// PV-11 (§15.5; `H:` §6 "fetchedAt = lastAttemptAt"): a later attempt that did not produce a snapshot — not current.
#[test]
fn a_source_tried_since_its_last_snapshot_is_not_current() {
    let retried = SourceFreshness {
        fetched_at: Some(minutes_ago(10)),
        last_attempt_at: Some(minutes_ago(5)),
        error: None,
    };
    assert!(!retried.is_current(now()));
}

/// PV-11 (§15.5; `H:` §6 "ไม่ใช่เวลาอนาคต"): a snapshot dated in the future is not current.
#[test]
fn a_source_fetched_in_the_future_is_not_current() {
    let future = SourceFreshness {
        fetched_at: Some(now() + Duration::minutes(1)),
        last_attempt_at: Some(now() + Duration::minutes(1)),
        error: None,
    };
    assert!(!future.is_current(now()));
}

/// PV-11 (§15.5): no snapshot, or never tried, is not current.
#[test]
fn a_source_with_no_snapshot_or_no_attempt_is_not_current() {
    let never = SourceFreshness {
        fetched_at: None,
        last_attempt_at: None,
        error: None,
    };
    assert!(!never.is_current(now()));
    let no_snapshot = SourceFreshness {
        fetched_at: None,
        ..fetched(1)
    };
    assert!(!no_snapshot.is_current(now()));
    let no_attempt = SourceFreshness {
        last_attempt_at: None,
        ..fetched(1)
    };
    assert!(!no_attempt.is_current(now()));
}

// ── FollowPlan::plan ───────────────────────────────────────────────────────────────────────────────────────────

/// PV-13 (§15.5; `H:` §4.4, §6): a fresh TT 33.20 / 33.90 at d = 2 with the defaults, within 1 % of the last tier set
/// and of the publication (tier 1 buy 33.23 vs 33.10 = +0.39 %, sell 33.87 vs 33.95 = −0.24 %), is a candidate with
/// exactly PV-1's 4 tiers and the TT anchor (its prices, fetch time and batch).
#[test]
fn a_fresh_tt_within_one_percent_is_a_candidate_with_v9s_tiers() {
    let mut input = published(usd_fetched(5), "33.10", "33.95");
    input.last_tier1 = Some(tier("33.10", "33.95"));
    assert_eq!(
        plan(&input),
        FollowOutcome::Candidate(
            pv1(),
            MarketAnchor {
                source: MarketSource::Tt,
                buy: rate("33.2"),
                sell: rate("33.9"),
                fetched_at: minutes_ago(5),
                batch_id: Some(BATCH),
            }
        )
    );
}

/// PV-13 (§15.5): the first plan, with nothing approved or published, is a candidate with PV-1's tiers.
#[test]
fn a_first_plan_with_nothing_published_is_a_candidate() {
    match plan(&usd_fetched(5)) {
        FollowOutcome::Candidate(tiers, anchor) => {
            assert_eq!(tiers, pv1());
            assert_eq!(anchor.batch_id, Some(BATCH));
        }
        other => panic!("expected Candidate, got {other:?}"),
    }
}

/// PV-11 (§15.5; `H:` §6): TT fetched 31 minutes ago blocks the plan with `STALE_SOURCE` alone.
#[test]
fn a_tt_fetched_thirty_one_minutes_ago_blocks_with_stale_source() {
    assert_eq!(
        plan(&usd_fetched(31)),
        FollowOutcome::Blocked(vec![FollowReason::StaleSource])
    );
}

/// PV-11 (§15.5): a failed last fetch blocks with `STALE_SOURCE` even though the snapshot is 2 minutes old.
#[test]
fn a_failed_last_fetch_blocks_with_stale_source() {
    let mut input = usd_fetched(2);
    input.source.error = Some("HTTP 503".to_string());
    assert_eq!(
        plan(&input),
        FollowOutcome::Blocked(vec![FollowReason::StaleSource])
    );
}

/// PV-12 (§15.5; `H:` §6): tier 1 buy 33.23 is +1.22 % over both the last tier set's and the published tier 1
/// (32.83; derived: (33.23 − 32.83) ÷ 32.83 × 100 = 1.218 %). Blocked with `LARGE_MOVE`, once.
#[test]
fn a_twelve_tenths_percent_buy_move_blocks_with_large_move() {
    let mut input = published(usd_fetched(5), "32.83", "33.87");
    input.last_tier1 = Some(tier("32.83", "33.87"));
    // reading: the reason is listed once although the suggestion's flag and the plan's own check both find it.
    assert_eq!(
        plan(&input),
        FollowOutcome::Blocked(vec![FollowReason::LargeMove])
    );
}

/// PV-12 (§15.5; `H:` §6 "ตรวจ large move ทั้ง buy/sell"): a sell-only move. Tier 1 sell 33.87 is −1.25 % under the
/// last tier set's 34.30 (derived: (33.87 − 34.30) ÷ 34.30 × 100 = −1.254 %); buy unchanged. The suggestion's own
/// `LargeMove` looks at buy only, so this is the plan's check.
#[test]
fn a_sell_only_move_blocks_with_large_move() {
    let mut input = usd_fetched(5);
    input.last_tier1 = Some(tier("33.23", "34.30"));
    assert_eq!(
        plan(&input),
        FollowOutcome::Blocked(vec![FollowReason::LargeMove])
    );
}

/// PV-12 (§15.5; `H:` §6 "เทียบทั้ง tier ที่เคยอนุมัติและราคา ledger ปัจจุบัน"): a move against the published tier 1 only
/// (sell 33.87 vs 34.30, −1.25 %), the last tier set unchanged, blocks.
#[test]
fn a_move_against_the_publication_only_blocks_with_large_move() {
    let mut input = published(usd_fetched(5), "33.23", "34.30");
    input.last_tier1 = Some(tier("33.23", "33.87"));
    assert_eq!(
        plan(&input),
        FollowOutcome::Blocked(vec![FollowReason::LargeMove])
    );
}

/// PV-12 (§15.5): a move against the last tier set only (buy 33.23 vs 32.83, +1.22 %), the publication unchanged,
/// blocks.
#[test]
fn a_move_against_the_last_tier_set_only_blocks_with_large_move() {
    let mut input = published(usd_fetched(5), "33.23", "33.87");
    input.last_tier1 = Some(tier("32.83", "33.87"));
    assert_eq!(
        plan(&input),
        FollowOutcome::Blocked(vec![FollowReason::LargeMove])
    );
}

/// PV-11 / PV-12 (follow_plan.rs: "every reason found, in `FollowReason` order"): stale and a large move together.
#[test]
fn every_reason_is_listed_in_order() {
    let mut input = usd_fetched(45);
    input.last_tier1 = Some(tier("32.83", "33.87"));
    assert_eq!(
        plan(&input),
        FollowOutcome::Blocked(vec![FollowReason::StaleSource, FollowReason::LargeMove])
    );
}

/// PV-11 (§15.5 `NO_ANCHOR`): no market quote blocks with `NO_ANCHOR`.
#[test]
fn no_market_quote_blocks_with_no_anchor() {
    let mut input = usd_fetched(5);
    input.suggestion.quotes.clear();
    let r = reasons(plan(&input));
    assert!(r.contains(&FollowReason::NoAnchor), "{r:?}");
}

/// PV-4 / PV-13 (§15.5 "It does not depend on the BoT tick: gap 5"; gap 7): with a TT anchor and no BoT base the plan
/// is still a candidate.
// reading: `NoAnchor` means no *market* anchor (`H:` §6; V9's NO_ANCHOR). The v3 suggestion also raises
// `PricingFlag::NoAnchor` when the BoT base is missing (§5.2, `pricing_suggestion.rs`), but FOLLOW prices from the
// market alone; a currency with no BoT line must still follow.
#[test]
fn follow_needs_a_market_anchor_not_a_bot_base() {
    let mut input = usd_fetched(5);
    input.suggestion.base_buy = None;
    input.suggestion.base_sell = None;
    match plan(&input) {
        FollowOutcome::Candidate(tiers, _) => assert_eq!(tiers, pv1()),
        other => panic!("expected Candidate, got {other:?}"),
    }
}

/// PV-12 (§15.5 "`BELOW_COST` … in the suggestion's flags"; `H:` §6): a partner cost of 33.85 (above tier 4's sell
/// 33.80, `pricing_suggestion_test.rs`) blocks with `BELOW_COST`.
#[test]
fn below_cost_blocks() {
    let mut input = usd_fetched(5);
    input.suggestion.partner_cost = Some(d("33.85"));
    let r = reasons(plan(&input));
    assert!(r.contains(&FollowReason::BelowCost), "{r:?}");
}

/// PV-12 (§15.5 "`PRECISION` … in the suggestion's flags"; `H:` §4.3): IDR 0.00178 / 0.0021 cannot be expressed in 4
/// decimals (`pricing_suggestion_test.rs`'s rupiah case) and blocks with `PRECISION`.
#[test]
fn precision_blocks() {
    let mut input = usd_fetched(5);
    input.suggestion.currency = CurrencyCode::parse("IDR").expect("currency");
    input.suggestion.quotes = vec![quote(MarketSource::Tt, "0.00178", "0.0021", minutes_ago(5))];
    input.suggestion.tt_decimals = Some(5);
    input.suggestion.base_buy = Some(rate("0.00178"));
    input.suggestion.base_sell = Some(rate("0.0021"));
    let r = reasons(plan(&input));
    assert!(r.contains(&FollowReason::Precision), "{r:?}");
}
