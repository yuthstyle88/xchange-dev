//! PT-2 (pricing-tiers.md §3, §10.1, §11): `CurrencyTierOverride::new` and `PolicyTiers::for_currency`.
//! Written before the implementation by the ticket author (claude/647e1967, PT-0). Do not edit, delete or weaken;
//! add tests in a file of your own. Pure: no database.

use xc_kernel::money::{CurrencyCode, Money, RatePercent, Side};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::policy::anchor_source::AnchorSource;
use xc_rate::domain::policy::currency_tier_override::{CurrencyTierOverride, TierMargin};
use xc_rate::domain::policy::policy_tiers::{PolicyTier, PolicyTiers};

fn pct(text: &str) -> RatePercent {
    RatePercent::parse(text).expect("a percent")
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("a currency")
}

fn margin(tier_no: u8, buy: &str, sell: &str) -> TierMargin {
    TierMargin {
        tier_no,
        buy_margin_pct: pct(buy),
        sell_margin_pct: pct(sell),
    }
}

fn four() -> Vec<TierMargin> {
    vec![
        margin(1, "-0.5000", "0.5000"),
        margin(2, "-0.4000", "0.4000"),
        margin(3, "-0.3000", "0.3000"),
        margin(4, "-0.2000", "0.2000"),
    ]
}

fn money(text: &str) -> Money {
    Money::parse(text).expect("money")
}

/// The branch's tiers: bands 10 000 / 50 000 / 200 000 THB, margins ±1, ±0.8, ±0.6, ±0.4.
fn branch_tiers() -> PolicyTiers {
    PolicyTiers::new(vec![
        PolicyTier::new(1, Some(money("10000")), pct("-1.0000"), pct("1.0000")),
        PolicyTier::new(2, Some(money("50000")), pct("-0.8000"), pct("0.8000")),
        PolicyTier::new(3, Some(money("200000")), pct("-0.6000"), pct("0.6000")),
        PolicyTier::new(4, None, pct("-0.4000"), pct("0.4000")),
    ])
    .expect("valid branch tiers")
}

fn is_validation(result: Result<CurrencyTierOverride, RateEngineError>, case: &str) {
    match result {
        Err(RateEngineError::ValidationFailed(_)) => {}
        other => panic!("{case}: expected ValidationFailed, got {other:?}"),
    }
}

#[test]
fn four_tiers_build_an_override_whose_margins_read_back_per_tier_and_side() {
    let o = CurrencyTierOverride::new(
        usd(),
        four(),
        Some(pct("1.5000")),
        Some(AnchorSource::MarketTt),
    )
    .expect("four tiers 1..=4 are valid");
    assert_eq!(o.currency, usd());
    assert_eq!(o.min_spread_pct, Some(pct("1.5000")));
    assert_eq!(o.anchor_source, Some(AnchorSource::MarketTt));
    assert_eq!(o.margin(1, Side::Buy), Some(pct("-0.5000")));
    assert_eq!(o.margin(1, Side::Sell), Some(pct("0.5000")));
    assert_eq!(o.margin(4, Side::Buy), Some(pct("-0.2000")));
    assert_eq!(o.margin(4, Side::Sell), Some(pct("0.2000")));
}

#[test]
fn tiers_given_out_of_order_are_stored_in_tier_order() {
    let mut shuffled = four();
    shuffled.reverse();
    shuffled.swap(1, 2);
    let o = CurrencyTierOverride::new(usd(), shuffled, None, None).expect("valid");
    let tiers = o.tiers.expect("four tiers");
    let order: Vec<u8> = tiers.iter().map(|t| t.tier_no).collect();
    assert_eq!(order, vec![1, 2, 3, 4]);
    assert_eq!(o.margin(2, Side::Sell), Some(pct("0.4000")));
}

#[test]
fn no_margins_is_an_override_of_the_minimum_spread_or_the_anchor_only() {
    let o = CurrencyTierOverride::new(usd(), vec![], Some(pct("0.0000")), None).expect("valid");
    assert_eq!(o.tiers, None);
    assert_eq!(o.margin(1, Side::Buy), None);
    assert_eq!(o.min_spread_pct, Some(pct("0.0000")));

    let o = CurrencyTierOverride::new(usd(), vec![], None, Some(AnchorSource::MarketSr))
        .expect("valid");
    assert_eq!(o.tiers, None);
    assert_eq!(o.anchor_source, Some(AnchorSource::MarketSr));
}

#[test]
fn three_tiers_are_refused() {
    let mut three = four();
    three.pop();
    is_validation(
        CurrencyTierOverride::new(usd(), three, None, None),
        "3 tiers (§11 row 'override with 3 rows')",
    );
}

#[test]
fn five_tiers_or_a_repeated_tier_are_refused() {
    let mut five = four();
    five.push(margin(4, "-0.1000", "0.1000"));
    is_validation(
        CurrencyTierOverride::new(usd(), five, None, None),
        "5 tiers",
    );

    let repeated = vec![
        margin(1, "-0.5000", "0.5000"),
        margin(1, "-0.4000", "0.4000"),
        margin(3, "-0.3000", "0.3000"),
        margin(4, "-0.2000", "0.2000"),
    ];
    is_validation(
        CurrencyTierOverride::new(usd(), repeated, None, None),
        "tier 1 twice, no tier 2",
    );
}

#[test]
fn a_tier_number_outside_one_to_four_is_refused() {
    let mut zero = four();
    zero[0].tier_no = 0;
    is_validation(CurrencyTierOverride::new(usd(), zero, None, None), "tier 0");

    let mut five = four();
    five[3].tier_no = 5;
    is_validation(CurrencyTierOverride::new(usd(), five, None, None), "tier 5");
}

#[test]
fn the_minimum_spread_lies_in_zero_to_five() {
    CurrencyTierOverride::new(usd(), vec![], Some(pct("0.0000")), None).expect("0 is allowed");
    CurrencyTierOverride::new(usd(), vec![], Some(pct("5.0000")), None).expect("5 is allowed");
    is_validation(
        CurrencyTierOverride::new(usd(), vec![], Some(pct("5.0001")), None),
        "min spread 5.0001",
    );
    is_validation(
        CurrencyTierOverride::new(usd(), vec![], Some(pct("-0.0001")), None),
        "min spread -0.0001",
    );
}

#[test]
fn for_currency_without_an_override_is_the_branch_tiers() {
    let branch = branch_tiers();
    assert_eq!(branch.for_currency(None), branch);
}

#[test]
fn for_currency_with_margins_keeps_the_bands_and_takes_the_currency_margins() {
    let branch = branch_tiers();
    let o = CurrencyTierOverride::new(usd(), four(), None, None).expect("valid");
    let resolved = branch.for_currency(Some(&o));

    assert_eq!(
        resolved.bands(),
        branch.bands(),
        "bands stay per branch (§3 [required])"
    );
    for (n, buy, sell) in [
        (1, "-0.5000", "0.5000"),
        (2, "-0.4000", "0.4000"),
        (3, "-0.3000", "0.3000"),
        (4, "-0.2000", "0.2000"),
    ] {
        assert_eq!(resolved.margin(n, Side::Buy), pct(buy), "tier {n} buy");
        assert_eq!(resolved.margin(n, Side::Sell), pct(sell), "tier {n} sell");
    }
    // The branch's own tiers are not changed by resolving one currency.
    assert_eq!(branch.margin(1, Side::Buy), pct("-1.0000"));
}

#[test]
fn for_currency_with_an_override_without_margins_is_the_branch_tiers() {
    let branch = branch_tiers();
    let o = CurrencyTierOverride::new(
        usd(),
        vec![],
        Some(pct("1.0000")),
        Some(AnchorSource::MarketTt),
    )
    .expect("valid");
    assert_eq!(branch.for_currency(Some(&o)), branch);
}
