//! XC-V9-PRICING PV-7 (pricing-tiers.md §15.4; `H:` §7; V9:engine `resolvePolicy`, `adjustTiers`): how a branch's
//! basis points resolve (currency row, then `'*'`, then 0; `own`/`follow` from the currency row only), how MAIN's
//! tiers are adjusted into a branch (buy down, sell up, at the base's decimals, at most 5; then monotonic; then
//! `TierCheck` against the anchor), and the −300..=300 bp limit. `H:` is the owner's handoff `PRICING-DESIGN.md`
//! and `V9:` its source excerpt (both kept out of the repo). Written before the implementation by the sub-controller
//! claude/451f0116 (XC-V9-PRICING skeleton). Do not edit, delete or weaken; add tests in a file of your own. Pure: no
//! database, no clock.

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use xc_kernel::money::Rate;
use xc_rate::domain::pricing::branch_policy::{
    BranchPolicyRow, BranchPricingPolicy, PolicyFrom, ResolvedPolicy,
};
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_suggestion::SuggestionConfig;
use xc_rate::domain::pricing::tier_check::TierError;
use xc_rate::domain::pricing::tier_set::TierRate;

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("rate")
}

fn tiers(list: [(&str, &str); 4]) -> [TierRate; 4] {
    list.map(|(buy, sell)| TierRate {
        buy: rate(buy),
        sell: rate(sell),
    })
}

/// PV-1's tiers (`H:` §4.4): MAIN's USD set.
fn main_usd() -> [TierRate; 4] {
    tiers([
        ("33.23", "33.87"),
        ("33.24", "33.86"),
        ("33.26", "33.84"),
        ("33.29", "33.80"),
    ])
}

/// TT USD(L) 33.20 / 33.90 per one unit (`H:` §4.4).
fn tt(buy: &str, sell: &str) -> MarketQuote {
    MarketQuote {
        source: MarketSource::Tt,
        buy: Some(rate(buy)),
        sell: Some(rate(sell)),
        unit_size: Decimal::ONE,
        fetched_at: Utc.with_ymd_and_hms(2026, 10, 27, 3, 0, 0).unwrap(),
        stale: false,
    }
}

fn bp(buy_bp: i32, sell_bp: i32) -> ResolvedPolicy {
    ResolvedPolicy {
        buy_bp,
        sell_bp,
        own: false,
        follow: false,
        from: PolicyFrom::Currency,
    }
}

fn adjust(
    base: &[TierRate; 4],
    policy: ResolvedPolicy,
    unit_size: &str,
    anchor: Option<&MarketQuote>,
) -> Result<[TierRate; 4], Vec<TierError>> {
    BranchPricingPolicy::adjust(
        base,
        &policy,
        d(unit_size),
        anchor,
        &SuggestionConfig::v9_default(),
    )
}

fn row(
    currency: &str,
    buy_bp: Option<i32>,
    sell_bp: Option<i32>,
    own: bool,
    follow: bool,
) -> BranchPolicyRow {
    BranchPolicyRow {
        currency: currency.to_string(),
        buy_bp,
        sell_bp,
        own,
        follow,
    }
}

/// V9 `test/pricing.mjs:163`'s branch B2: `'*'` −10/+15, EUR sells +30, JPY is its own price.
fn b2() -> BranchPricingPolicy {
    BranchPricingPolicy {
        rows: vec![
            row("*", Some(-10), Some(15), false, false),
            row("EUR", None, Some(30), false, false),
            row("JPY", None, None, true, false),
        ],
    }
}

fn resolved(
    buy_bp: i32,
    sell_bp: i32,
    own: bool,
    follow: bool,
    from: PolicyFrom,
) -> ResolvedPolicy {
    ResolvedPolicy {
        buy_bp,
        sell_bp,
        own,
        follow,
        from,
    }
}

// ── resolve ────────────────────────────────────────────────────────────────────────────────────────────────────

/// PV-7 (§15.4 resolution; V9 `test/pricing.mjs:164`): a currency with no row of its own takes the branch `'*'` row.
#[test]
fn a_currency_without_a_row_takes_the_branch_default() {
    assert_eq!(
        b2().resolve("USD"),
        resolved(-10, 15, false, false, PolicyFrom::Branch)
    );
}

/// PV-7 (§15.4; V9 `test/pricing.mjs:165`): a currency row overrides one side only; the NULL side falls back to `'*'`.
#[test]
fn a_currency_row_overrides_only_the_side_it_sets() {
    assert_eq!(
        b2().resolve("EUR"),
        resolved(-10, 30, false, false, PolicyFrom::Currency)
    );
}

/// PV-8 (§15.4; V9 `test/pricing.mjs:166`): `own` comes from the currency row; the bps still resolve (`from` is
/// `'*'` because the JPY row sets neither bp).
#[test]
fn own_comes_from_the_currency_row() {
    assert_eq!(
        b2().resolve("JPY"),
        resolved(-10, 15, true, false, PolicyFrom::Branch)
    );
}

/// PV-7 (§15.4; V9 `test/pricing.mjs:167`): a branch with no rows follows MAIN unchanged.
#[test]
fn a_branch_with_no_rows_follows_main_unchanged() {
    assert_eq!(
        BranchPricingPolicy::default().resolve("USD"),
        resolved(0, 0, false, false, PolicyFrom::Default)
    );
}

/// PV-14 (§15.2 `follow`; `H:` §7 last bullet; §10.7): a `'*'` row's `own` and `follow` never apply to a currency —
/// a branch default never enrols FOLLOW and never makes a price the branch's own.
#[test]
fn the_branch_default_never_enrols_follow_or_own() {
    let policy = BranchPricingPolicy {
        rows: vec![row("*", Some(5), Some(-5), true, true)],
    };
    assert_eq!(
        policy.resolve("USD"),
        resolved(5, -5, false, false, PolicyFrom::Branch)
    );
}

/// PV-14 (§15.4; `H:` §7): an explicit currency row enrols FOLLOW; with no bp on either row the pair is 0/0 from
/// `default`.
#[test]
fn a_currency_row_enrols_follow() {
    let policy = BranchPricingPolicy {
        rows: vec![row("USD", None, None, false, true)],
    };
    assert_eq!(
        policy.resolve("USD"),
        resolved(0, 0, false, true, PolicyFrom::Default)
    );
    assert!(!policy.resolve("EUR").follow, "only USD is enrolled");
}

/// PV-7 (§15.4): a currency row that sets one bp with no `'*'` row: that side from the row, the other 0, `from`
/// `currency`.
#[test]
fn one_side_on_a_currency_row_and_no_branch_default() {
    let policy = BranchPricingPolicy {
        rows: vec![row("USD", Some(5), None, false, false)],
    };
    assert_eq!(
        policy.resolve("USD"),
        resolved(5, 0, false, false, PolicyFrom::Currency)
    );
}

// ── adjust ─────────────────────────────────────────────────────────────────────────────────────────────────────

/// PV-7 (`H:` §7: base from §4, buyBp +5 / sellBp −5 ⇒ tier 1 = 33.24/33.86 at 2 decimals). Tiers 2–4 are derived
/// from H: §7's formula:
/// - tier 1: 33.23 × 1.0005 = 33.246615 → floor 33.24 · 33.87 × 0.9995 = 33.853065 → ceil 33.86 (`H:` states this);
/// - tier 2: 33.24 × 1.0005 = 33.25662 → 33.25 · 33.86 × 0.9995 = 33.84307 → 33.85 (derived);
/// - tier 3: 33.26 × 1.0005 = 33.27663 → 33.27 · 33.84 × 0.9995 = 33.82308 → 33.83 (derived);
/// - tier 4: 33.29 × 1.0005 = 33.306645 → 33.30 · 33.80 × 0.9995 = 33.7831 → 33.79 (derived).
///
/// Already monotonic, and tier 1 is 0.12 % / 0.13 % from TT 33.20/33.90 (inside 3 %).
#[test]
fn main_plus_five_minus_five_bp_gives_v9s_branch_tiers() {
    let got = adjust(&main_usd(), bp(5, -5), "1", Some(&tt("33.20", "33.90")));
    assert_eq!(
        got,
        Ok(tiers([
            ("33.24", "33.86"),
            ("33.25", "33.85"),
            ("33.27", "33.83"),
            ("33.30", "33.79"),
        ]))
    );
}

/// PV-7 (V9 `test/pricing.mjs:155`): no adjustment is MAIN's set exactly.
#[test]
fn no_adjustment_is_mains_set_exactly() {
    assert_eq!(
        adjust(&main_usd(), bp(0, 0), "1", Some(&tt("33.20", "33.90"))),
        Ok(main_usd())
    );
}

/// PV-7 (V9 `test/pricing.mjs:156-158`): −10 / +15 bp ⇒ tier 1 33.23 × 0.999 = 33.19677 → 33.19 (buy down),
/// 33.87 × 1.0015 = 33.920805 → 33.93 (sell up); every tier keeps the order.
#[test]
fn buy_rounds_down_and_sell_rounds_up() {
    let got = adjust(&main_usd(), bp(-10, 15), "1", None).expect("in range");
    assert_eq!(got[0], tiers([("33.19", "33.93"); 4])[0]);
    for n in 1..4 {
        assert!(got[n].buy >= got[n - 1].buy, "buy tier {}: {got:?}", n + 1);
        assert!(
            got[n].sell <= got[n - 1].sell,
            "sell tier {}: {got:?}",
            n + 1
        );
    }
}

/// PV-7 (§15.4 "then monotonic"; `H:` §7 "branch adjustment ยังรักษาลำดับ tier"): a base whose tier 2 buys below and
/// sells above tier 1 comes out with tier 2 = tier 1 on both sides (`buy = max(buy, prev)`, `sell = min(sell, prev)`).
#[test]
fn the_adjustment_repairs_the_tier_order() {
    let base = tiers([
        ("33.23", "33.87"),
        ("33.22", "33.88"),
        ("33.26", "33.84"),
        ("33.29", "33.80"),
    ]);
    assert_eq!(
        adjust(&base, bp(0, 0), "1", None),
        Ok(tiers([
            ("33.23", "33.87"),
            ("33.23", "33.87"),
            ("33.26", "33.84"),
            ("33.29", "33.80"),
        ]))
    );
}

/// PV-3 / PV-7 (§15.4 "`d` … at most 5"; `H:` §4.3 "branch adjustment รองรับความละเอียดจากฐานสูงสุด 5 ตำแหน่ง"): a
/// VND-like 5-decimal base keeps 5 decimals. −20 / +20 bp, derived from H: §7's formula at d = 5:
/// - 0.00127 × 0.998 = 0.00126746 → 0.00126 · 0.00133 × 1.002 = 0.00133266 → 0.00134;
/// - 0.00128 × 0.998 = 0.00127744 → 0.00127 · 0.00132 × 1.002 = 0.00132264 → 0.00133 (tiers 2 and 3);
/// - 0.00129 × 0.998 = 0.00128742 → 0.00128 · 0.00131 × 1.002 = 0.00131262 → 0.00132.
///
/// 4 decimals would give 0.0012 / 0.0014; 8 would keep 0.00126746.
#[test]
fn a_five_decimal_base_keeps_five_decimals() {
    let base = tiers([
        ("0.00127", "0.00133"),
        ("0.00128", "0.00132"),
        ("0.00128", "0.00132"),
        ("0.00129", "0.00131"),
    ]);
    let got = adjust(&base, bp(-20, 20), "1", None).expect("in range");
    assert_eq!(
        got,
        tiers([
            ("0.00126", "0.00134"),
            ("0.00127", "0.00133"),
            ("0.00127", "0.00133"),
            ("0.00128", "0.00132"),
        ])
    );
    for t in got {
        assert!(t.buy.value().normalize().scale() <= 5, "{t:?}");
        assert!(t.sell.value().normalize().scale() <= 5, "{t:?}");
    }
}

/// PV-7 with B1 (§15.0 "every price … is per `unit_size`"; V9 `test/pricing.mjs:160-161`): JPY per 100, −20 / +20 bp.
/// V9 pins tier 1 per one yen at 0.2082 / 0.2178, so per 100 it is 20.82 / 21.78; the anchor stays per one yen
/// (TT 0.2085 / 0.2175) and is compared through `unit_size`. The base is V9's JPY suggestion × 100, derived from H:
/// §4.2's formula with d = 4 (0.2087/0.2173 · 0.2087/0.2172 · 0.2089/0.2171 · 0.2091/0.2169).
#[test]
fn yen_per_hundred_adjusts_like_v9s_per_yen_price_times_a_hundred() {
    let base = tiers([
        ("20.87", "21.73"),
        ("20.87", "21.72"),
        ("20.89", "21.71"),
        ("20.91", "21.69"),
    ]);
    let got = adjust(&base, bp(-20, 20), "100", Some(&tt("0.2085", "0.2175")))
        .expect("tier 1 is 0.14 % from TT");
    assert_eq!(got[0], tiers([("20.82", "21.78"); 4])[0]);
}

/// PV-7 (§15.4 "then `TierCheck::validate` against the anchor"; V9 `test/pricing.mjs:177-179`): the market moved after
/// MAIN approved — −50 bp gives tier 1 buy 33.23 × 0.995 = 33.06385 → 33.06, 3.9 % under TT 34.40 — refused although
/// the adjustment is in range.
#[test]
fn an_adjusted_set_far_from_the_anchor_is_refused() {
    let got = adjust(&main_usd(), bp(-50, 0), "1", Some(&tt("34.40", "35.10")));
    let errors = got.expect_err("tier 1 is more than 3 % from TT");
    assert!(
        errors.contains(&TierError::FarFromAnchor {
            source: MarketSource::Tt,
            limit_pct: d("3"),
        }),
        "{errors:?}"
    );
}

/// PV-7 (V9 `test/pricing.mjs:174-175`): +300 / −300 bp crosses buy and sell (tier 1 buy 33.23 × 1.03 = 34.2269 →
/// 34.22, sell 33.87 × 0.97 = 32.8539 → 32.86) and is refused.
#[test]
fn an_adjustment_that_crosses_buy_and_sell_is_refused() {
    let errors = adjust(&main_usd(), bp(300, -300), "1", None).expect_err("sell below buy");
    assert!(
        errors.contains(&TierError::SellNotAboveBuy { tier: 1 }),
        "{errors:?}"
    );
}

// ── check_bp ───────────────────────────────────────────────────────────────────────────────────────────────────

/// PV-7 (§15.4 "A bp outside −300..300 is `validationFailed`"; `H:` §7; V9 `validateAdjust`): ±300 is accepted, ±301
/// refused.
#[test]
fn a_bp_is_between_minus_and_plus_three_hundred() {
    assert_eq!(BranchPricingPolicy::check_bp(-300), Ok(-300));
    assert_eq!(BranchPricingPolicy::check_bp(0), Ok(0));
    assert_eq!(BranchPricingPolicy::check_bp(300), Ok(300));
    assert!(BranchPricingPolicy::check_bp(301).is_err());
    assert!(BranchPricingPolicy::check_bp(-301).is_err());
}

/// G-11 (owner-set-values.md §2 row 8): `check_bp_with` uses the Owner's `branch_bp_cap` and refuses `None` with
/// `pricingRuleNotSet` naming `branchBpCap`.
#[test]
fn g11_check_bp_with_uses_owner_cap_and_refuses_not_set() {
    assert_eq!(
        BranchPricingPolicy::check_bp_with(-150, Some(150)).unwrap(),
        -150
    );
    assert_eq!(BranchPricingPolicy::check_bp_with(0, Some(150)).unwrap(), 0);
    assert_eq!(
        BranchPricingPolicy::check_bp_with(150, Some(150)).unwrap(),
        150
    );
    assert_eq!(
        BranchPricingPolicy::check_bp_with(151, Some(150))
            .unwrap_err()
            .code(),
        "validationFailed"
    );
    assert_eq!(
        BranchPricingPolicy::check_bp_with(-151, Some(150))
            .unwrap_err()
            .code(),
        "validationFailed"
    );
    let err = BranchPricingPolicy::check_bp_with(5, None).unwrap_err();
    assert_eq!(err.code(), "pricingRuleNotSet");
    assert_eq!(err.to_string(), "branchBpCap");
    assert_eq!(
        BranchPricingPolicy::check_bp(301),
        Err("bp must be between -300 and 300".to_string())
    );
}

#[test]
fn adjust_with_refuses_when_far_from_anchor_limit_is_not_set() {
    let err = BranchPricingPolicy::adjust_with(
        &main_usd(),
        &bp(5, -5),
        d("1"),
        Some(&tt("33.20", "33.90")),
        None,
    )
    .unwrap_err();
    assert_eq!(err.field, "farFromAnchorLimitPct");
}
