//! Ticket #1489 (VT-2) — Pure PolicyTiers and TierProposal::of (rates.md §12.3, §12.10).
//! Written before the implementation by the test writer gemini/pool-6 (ordered by claude/ad05bf8d).
//!
//! Design: `docs/design/blueprint/rates.md` §12.3 (additive margin formula, worked example to the digit,
//! 4 tiers T1..T4, bands and margins), §12.10 VT-2 row at line 1731, O-7 (margins same for every currency),
//! O-9 (additive, one multiplication and one rounding).
//!
//! Pure domain tests: no database required.

use serde_json::json;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, MinorUnit, Money, Rate, RatePercent, Side, UnitSize,
};
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::draft::rate_proposal::{ProposedLine, RateProposal};
use xc_rate::domain::draft::tier_proposal::{TierLine, TierProposal};
use xc_rate::domain::policy::policy_tiers::{PolicyTier, PolicyTiers};
use xc_rate::domain::policy::rate_policy::RatePolicy;
use xc_rate::domain::publication::value_tier::{TierBand, TierCode};
use xc_rate::domain::reference::reference_batch::ReferenceLine;

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known code")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a valid rate")
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).expect("a valid unit size")
}

fn pct(text: &str) -> RatePercent {
    RatePercent::parse(text).expect("a valid percent")
}

fn money(text: &str) -> Money {
    Money::parse(text).expect("a valid money amount")
}

fn tier_code(text: &str) -> TierCode {
    TierCode::new(text).expect("a valid tier code")
}

fn units(currency: &str, unit_size: &str) -> CurrencyUnits {
    CurrencyUnits::new(code(currency), unit(unit_size), MinorUnit::TWO)
}

fn reference(
    currency: &str,
    ref_unit: &str,
    buy: Option<&str>,
    sell: Option<&str>,
) -> ReferenceLine {
    ReferenceLine::new(
        code(currency),
        unit(ref_unit),
        buy.map(rate),
        sell.map(rate),
    )
}

fn policy(buy: &str, sell: &str) -> RatePolicy {
    let wide = json!({ "min_pct": "-50.0000", "max_pct": "50.0000" });
    let guards = json!({
        "USD": { "buy": wide, "sell": wide },
        "JPY": { "buy": wide, "sell": wide },
    });
    RatePolicy::new("policy-under-test", pct(buy), pct(sell), &guards).expect("a valid policy")
}

fn example_tiers() -> PolicyTiers {
    PolicyTiers::new(vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("+0.1000"), pct("-0.1000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("+0.2000"), pct("-0.2000")),
        PolicyTier::new(4, None, pct("+0.3000"), pct("-0.3000")),
    ])
    .expect("valid example policy tiers")
}

// ── §9 Row 1: §12.3 worked example to the digit ──────────────────────────────

#[test]
fn worked_example_usd_digits_match_section_12_3() {
    // rates.md §12.3 worked example:
    // USD, unit 1; BoT buy 32.4500, sell 32.9000; buy_adjust_pct -0.5000, sell_adjust_pct +0.4000
    // Margins: T1 (0, 0), T2 (+0.1, -0.1), T3 (+0.2, -0.2), T4 (+0.3, -0.3)
    let ref_line = reference("USD", "1", Some("32.4500"), Some("32.9000"));
    let pol = policy("-0.5000", "+0.4000");
    let u = units("USD", "1");
    let draft_line = DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None);

    let proposed: ProposedLine = RateProposal::of(&draft_line, Some(&ref_line), Some(&pol), &u)
        .expect("rate proposal succeeds");
    assert_eq!(proposed.buy, Some(rate("32.28775000")));
    assert_eq!(proposed.sell, Some(rate("33.03160000")));

    let tiers = example_tiers();
    let tier_lines: Vec<TierLine> = TierProposal::of(
        &proposed,
        &draft_line,
        Some(&ref_line),
        Some(&pol),
        &tiers,
        &u,
    )
    .expect("tier proposal succeeds");

    assert_eq!(tier_lines.len(), 4, "must produce exactly 4 tier lines");

    // T1: band <= 20,000.00, margin 0.0000 / 0.0000
    // buy: 32.45 * 0.9950 = 32.28775000, sell: 32.90 * 1.0040 = 33.03160000
    let t1 = &tier_lines[0];
    assert_eq!(t1.code, tier_code("T1"));
    assert_eq!(
        t1.band,
        TierBand {
            code: tier_code("T1"),
            lower_exclusive_thb: None,
            upper_inclusive_thb: Some(money("20000.00")),
        }
    );
    assert_eq!(t1.buy, Some(rate("32.28775000")), "T1 buy rate");
    assert_eq!(t1.sell, Some(rate("33.03160000")), "T1 sell rate");
    assert_eq!(t1.base_buy, Some(rate("32.28775000")), "T1 base buy rate");
    assert_eq!(t1.base_sell, Some(rate("33.03160000")), "T1 base sell rate");

    // T2: band (20,000.00, 100,000.00], margin +0.1000 / -0.1000
    // buy: 32.45 * 0.9960 = 32.32020000, sell: 32.90 * 1.0030 = 32.99870000
    let t2 = &tier_lines[1];
    assert_eq!(t2.code, tier_code("T2"));
    assert_eq!(
        t2.band,
        TierBand {
            code: tier_code("T2"),
            lower_exclusive_thb: Some(money("20000.00")),
            upper_inclusive_thb: Some(money("100000.00")),
        }
    );
    assert_eq!(t2.buy, Some(rate("32.32020000")), "T2 buy rate");
    assert_eq!(t2.sell, Some(rate("32.99870000")), "T2 sell rate");
    assert_eq!(t2.base_buy, Some(rate("32.28775000")), "T2 base buy rate");
    assert_eq!(t2.base_sell, Some(rate("33.03160000")), "T2 base sell rate");

    // T3: band (100,000.00, 500,000.00], margin +0.2000 / -0.2000
    // buy: 32.45 * 0.9970 = 32.35265000, sell: 32.90 * 1.0020 = 32.96580000
    let t3 = &tier_lines[2];
    assert_eq!(t3.code, tier_code("T3"));
    assert_eq!(
        t3.band,
        TierBand {
            code: tier_code("T3"),
            lower_exclusive_thb: Some(money("100000.00")),
            upper_inclusive_thb: Some(money("500000.00")),
        }
    );
    assert_eq!(t3.buy, Some(rate("32.35265000")), "T3 buy rate");
    assert_eq!(t3.sell, Some(rate("32.96580000")), "T3 sell rate");
    assert_eq!(t3.base_buy, Some(rate("32.28775000")), "T3 base buy rate");
    assert_eq!(t3.base_sell, Some(rate("33.03160000")), "T3 base sell rate");

    // T4: band > 500,000.00, margin +0.3000 / -0.3000
    // buy: 32.45 * 0.9980 = 32.38510000, sell: 32.90 * 1.0010 = 32.93290000
    let t4 = &tier_lines[3];
    assert_eq!(t4.code, tier_code("T4"));
    assert_eq!(
        t4.band,
        TierBand {
            code: tier_code("T4"),
            lower_exclusive_thb: Some(money("500000.00")),
            upper_inclusive_thb: None,
        }
    );
    assert_eq!(t4.buy, Some(rate("32.38510000")), "T4 buy rate");
    assert_eq!(t4.sell, Some(rate("32.93290000")), "T4 sell rate");
    assert_eq!(t4.base_buy, Some(rate("32.28775000")), "T4 base buy rate");
    assert_eq!(t4.base_sell, Some(rate("33.03160000")), "T4 base sell rate");

    // Verify PolicyTiers::bands() and PolicyTiers::margin()
    let bands = tiers.bands();
    assert_eq!(bands[0], t1.band);
    assert_eq!(bands[1], t2.band);
    assert_eq!(bands[2], t3.band);
    assert_eq!(bands[3], t4.band);

    assert_eq!(tiers.margin(1, Side::Buy), pct("0.0000"));
    assert_eq!(tiers.margin(1, Side::Sell), pct("0.0000"));
    assert_eq!(tiers.margin(2, Side::Buy), pct("+0.1000"));
    assert_eq!(tiers.margin(2, Side::Sell), pct("-0.1000"));
    assert_eq!(tiers.margin(3, Side::Buy), pct("+0.2000"));
    assert_eq!(tiers.margin(3, Side::Sell), pct("-0.2000"));
    assert_eq!(tiers.margin(4, Side::Buy), pct("+0.3000"));
    assert_eq!(tiers.margin(4, Side::Sell), pct("-0.3000"));
}

// ── §9 Row 2: MANUAL sides equal on all four tiers ───────────────────────────

#[test]
fn manual_side_unchanged_across_all_four_tiers() {
    let ref_line = reference("USD", "1", Some("32.4500"), Some("32.9000"));
    let pol = policy("-0.5000", "+0.4000");
    let u = units("USD", "1");
    let tiers = example_tiers();

    // One MANUAL side (Buy) and one AUTO side (Sell)
    let manual_buy_rate = rate("32.50000000");
    let line_one_manual = DraftLine::new(
        code("USD"),
        LineSide::Manual(manual_buy_rate),
        LineSide::Auto,
        Some("Owner manual override".into()),
    );
    let prop_one_manual = RateProposal::of(&line_one_manual, Some(&ref_line), Some(&pol), &u)
        .expect("proposal succeeds");

    let tier_lines = TierProposal::of(
        &prop_one_manual,
        &line_one_manual,
        Some(&ref_line),
        Some(&pol),
        &tiers,
        &u,
    )
    .expect("tier proposal succeeds");

    assert_eq!(tier_lines.len(), 4);
    for (i, tl) in tier_lines.iter().enumerate() {
        assert_eq!(
            tl.buy,
            Some(manual_buy_rate),
            "tier {} buy must remain the manual value without margin",
            i + 1
        );
        assert_eq!(
            tl.base_buy,
            Some(manual_buy_rate),
            "tier {} base_buy must match manual value",
            i + 1
        );
    }
    // Auto side varies across tiers
    assert_ne!(tier_lines[0].sell, tier_lines[3].sell);

    // Both sides MANUAL
    let manual_sell_rate = rate("32.85000000");
    let line_both_manual = DraftLine::new(
        code("USD"),
        LineSide::Manual(manual_buy_rate),
        LineSide::Manual(manual_sell_rate),
        Some("Both sides manual".into()),
    );
    let prop_both_manual = RateProposal::of(&line_both_manual, Some(&ref_line), Some(&pol), &u)
        .expect("proposal succeeds");

    let both_tier_lines = TierProposal::of(
        &prop_both_manual,
        &line_both_manual,
        Some(&ref_line),
        Some(&pol),
        &tiers,
        &u,
    )
    .expect("tier proposal succeeds");

    assert_eq!(both_tier_lines.len(), 4);
    for (i, tl) in both_tier_lines.iter().enumerate() {
        assert_eq!(
            tl.buy,
            Some(manual_buy_rate),
            "tier {} buy unchanged",
            i + 1
        );
        assert_eq!(
            tl.sell,
            Some(manual_sell_rate),
            "tier {} sell unchanged",
            i + 1
        );
        assert_eq!(
            tl.base_buy,
            Some(manual_buy_rate),
            "tier {} base_buy unchanged",
            i + 1
        );
        assert_eq!(
            tl.base_sell,
            Some(manual_sell_rate),
            "tier {} base_sell unchanged",
            i + 1
        );
    }
}

// ── §9 Row 3: Additive, not compounded ───────────────────────────────────────

#[test]
fn additive_margin_formula_refuses_compounding() {
    // Formula per §12.3 and O-9:
    // Additive: rate = round8( base * (1 + (adjust_pct + margin_pct) / 100) )
    // Compounded mutant: rate = round8( base * (1 + adjust_pct / 100) * (1 + margin_pct / 100) )
    //
    // For T4:
    // Base ref: buy 32.4500, sell 32.9000
    // Adjust: buy -0.5000%, sell +0.4000%
    // T4 margins: buy +0.3000%, sell -0.3000%
    //
    // Buy:
    // Additive:   32.45 * (1 + (-0.5000 + 0.3000)/100) = 32.45 * 0.9980 = 32.38510000
    // Compounded: 32.45 * (1 - 0.0050) * (1 + 0.0030) = 32.28775 * 1.003 = 32.38461325
    //
    // Sell:
    // Additive:   32.90 * (1 + (0.4000 - 0.3000)/100) = 32.90 * 1.0010 = 32.93290000
    // Compounded: 32.90 * (1 + 0.0040) * (1 - 0.0030) = 33.0316 * 0.997 = 32.93250520

    let ref_line = reference("USD", "1", Some("32.4500"), Some("32.9000"));
    let pol = policy("-0.5000", "+0.4000");
    let u = units("USD", "1");
    let draft_line = DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None);
    let proposed = RateProposal::of(&draft_line, Some(&ref_line), Some(&pol), &u).unwrap();
    let tiers = example_tiers();

    let tier_lines = TierProposal::of(
        &proposed,
        &draft_line,
        Some(&ref_line),
        Some(&pol),
        &tiers,
        &u,
    )
    .expect("tier proposal succeeds");

    let t4 = &tier_lines[3];

    // Must match additive results exactly
    assert_eq!(t4.buy, Some(rate("32.38510000")), "must be additive");
    assert_eq!(t4.sell, Some(rate("32.93290000")), "must be additive");

    // Must NOT match compounded results
    assert_ne!(
        t4.buy,
        Some(rate("32.38461325")),
        "compounded buy calculation must not pass"
    );
    assert_ne!(
        t4.sell,
        Some(rate("32.93250520")),
        "compounded sell calculation must not pass"
    );
}

// ── §9 Row 4: base_rate equals RateProposal::of for the same line ───────────

#[test]
fn base_rates_match_rate_proposal_of_for_every_tier() {
    let u = units("USD", "1");
    let pol = policy("-0.2500", "+0.3500");
    let tiers = example_tiers();
    let ref_line = reference("USD", "1", Some("35.0000"), Some("35.5000"));
    let draft_line = DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None);

    let proposed = RateProposal::of(&draft_line, Some(&ref_line), Some(&pol), &u)
        .expect("RateProposal succeeds");

    let tier_lines = TierProposal::of(
        &proposed,
        &draft_line,
        Some(&ref_line),
        Some(&pol),
        &tiers,
        &u,
    )
    .expect("TierProposal succeeds");

    assert_eq!(tier_lines.len(), 4);
    for (i, tl) in tier_lines.iter().enumerate() {
        assert_eq!(
            tl.base_buy,
            proposed.buy,
            "tier {} base_buy must equal RateProposal::of buy",
            i + 1
        );
        assert_eq!(
            tl.base_sell,
            proposed.sell,
            "tier {} base_sell must equal RateProposal::of sell",
            i + 1
        );
    }

    // Also verify for non-1 unit currency: JPY with unit 100
    let jpy_units = units("JPY", "100");
    let jpy_ref = reference("JPY", "100", Some("22.5000"), Some("23.1000"));
    let jpy_line = DraftLine::new(code("JPY"), LineSide::Auto, LineSide::Auto, None);
    let jpy_pol = policy("+0.1000", "-0.1000");

    let jpy_prop = RateProposal::of(&jpy_line, Some(&jpy_ref), Some(&jpy_pol), &jpy_units)
        .expect("JPY proposal succeeds");
    let jpy_tiers = TierProposal::of(
        &jpy_prop,
        &jpy_line,
        Some(&jpy_ref),
        Some(&jpy_pol),
        &tiers,
        &jpy_units,
    )
    .expect("JPY TierProposal succeeds");

    for (i, tl) in jpy_tiers.iter().enumerate() {
        assert_eq!(
            tl.base_buy,
            jpy_prop.buy,
            "JPY tier {} base_buy must match RateProposal::of",
            i + 1
        );
        assert_eq!(
            tl.base_sell,
            jpy_prop.sell,
            "JPY tier {} base_sell must match RateProposal::of",
            i + 1
        );
    }
}

// ── §9 Row 5: A tier where buy > sell after rounding is refused ──────────────

#[test]
fn refuses_tier_where_buy_rate_exceeds_sell_rate() {
    // Narrow spread reference: buy 30.0000, sell 30.1000
    let ref_line = reference("USD", "1", Some("30.0000"), Some("30.1000"));
    let pol = policy("0.0000", "0.0000");
    let u = units("USD", "1");
    let draft_line = DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None);
    let proposed = RateProposal::of(&draft_line, Some(&ref_line), Some(&pol), &u).unwrap();

    // T4 has +1.0000% buy margin, -1.0000% sell margin
    // T4 buy = 30.00 * 1.01 = 30.30000000
    // T4 sell = 30.10 * 0.99 = 29.79900000
    // After rounding, buy (30.30) > sell (29.799)
    let crossing_tiers = PolicyTiers::new(vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("+1.0000"), pct("-1.0000")),
    ])
    .expect("policy tiers definition itself is valid");

    let result = TierProposal::of(
        &proposed,
        &draft_line,
        Some(&ref_line),
        Some(&pol),
        &crossing_tiers,
        &u,
    );

    match result {
        Err(RateEngineError::ValidationFailed(msg)) => {
            assert!(
                msg.contains("buy") && msg.contains("sell"),
                "expected error message about buy <= sell, got: {msg}"
            );
        }
        other => panic!("expected ValidationFailed when buy > sell in a tier, got {other:?}"),
    }
}

// ── §9 Row 6: Bands not strictly rising and structure validations ────────────

#[test]
fn refuses_non_strictly_rising_bands() {
    // Equal uppers: tier 1 and tier 2 both 20,000.00
    let equal_rows = vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ];
    match PolicyTiers::new(equal_rows) {
        Err(RateEngineError::ValidationFailed(msg)) => {
            assert!(
                msg.to_lowercase().contains("rising")
                    || msg.to_lowercase().contains("upper")
                    || msg.to_lowercase().contains("band"),
                "expected rising bands error message, got: {msg}"
            );
        }
        other => panic!("expected ValidationFailed for equal upper bounds, got {other:?}"),
    }

    // Decreasing uppers: tier 1 is 50,000, tier 2 is 20,000
    let decreasing_rows = vec![
        PolicyTier::new(1, Some(money("50000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(decreasing_rows),
        Err(RateEngineError::ValidationFailed(_))
    ));

    // Zero upper bound is not > 0
    let zero_upper = vec![
        PolicyTier::new(1, Some(money("0.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(zero_upper),
        Err(RateEngineError::ValidationFailed(_))
    ));
}

#[test]
fn refuses_tier_4_with_upper_bound() {
    let rows = vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, Some(money("1000000.00")), pct("0.0000"), pct("0.0000")),
    ];
    match PolicyTiers::new(rows) {
        Err(RateEngineError::ValidationFailed(msg)) => {
            assert!(
                msg.contains("4") || msg.to_lowercase().contains("upper"),
                "expected tier 4 upper bound refusal, got: {msg}"
            );
        }
        other => panic!("expected ValidationFailed for tier 4 with upper bound, got {other:?}"),
    }
}

#[test]
fn refuses_tier_1_to_3_without_upper_bound() {
    // Tier 1 without upper bound
    let rows_t1_none = vec![
        PolicyTier::new(1, None, pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(rows_t1_none),
        Err(RateEngineError::ValidationFailed(_))
    ));

    // Tier 2 without upper bound
    let rows_t2_none = vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, None, pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(rows_t2_none),
        Err(RateEngineError::ValidationFailed(_))
    ));

    // Tier 3 without upper bound
    let rows_t3_none = vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, None, pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(rows_t3_none),
        Err(RateEngineError::ValidationFailed(_))
    ));
}

#[test]
fn refuses_row_count_not_equal_to_four() {
    // 3 rows
    let three_rows = vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(three_rows),
        Err(RateEngineError::ValidationFailed(_))
    ));

    // 5 rows
    let five_rows = vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, Some(money("1000000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(5, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(five_rows),
        Err(RateEngineError::ValidationFailed(_))
    ));

    // Empty rows
    assert!(matches!(
        PolicyTiers::new(vec![]),
        Err(RateEngineError::ValidationFailed(_))
    ));

    // 4 rows but tier_no not 1..=4 (e.g. duplicate tier_no 2)
    let bad_tier_nos = vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ];
    assert!(matches!(
        PolicyTiers::new(bad_tier_nos),
        Err(RateEngineError::ValidationFailed(_))
    ));
}

// ── Additional edge case: MidpointAwayFromZero rounding at 8th decimal place ──

#[test]
fn auto_tier_rate_rounds_half_up_away_from_zero() {
    // 32.125 * (1 + 0.0001/100) = 32.125032125
    // 8th digit is 2 (even), 9th digit is 5.
    // MidpointAwayFromZero -> 32.12503213
    // Banker's rounding (MidpointNearestEven) -> 32.12503212
    let ref_line = reference("USD", "1", Some("32.125"), Some("32.500"));
    let pol = policy("0.0000", "0.0000");
    let u = units("USD", "1");
    let draft_line = DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None);
    let proposed = RateProposal::of(&draft_line, Some(&ref_line), Some(&pol), &u).unwrap();

    let tiers = PolicyTiers::new(vec![
        PolicyTier::new(1, Some(money("20000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(2, Some(money("100000.00")), pct("+0.0001"), pct("0.0000")),
        PolicyTier::new(3, Some(money("500000.00")), pct("0.0000"), pct("0.0000")),
        PolicyTier::new(4, None, pct("0.0000"), pct("0.0000")),
    ])
    .unwrap();

    let tier_lines = TierProposal::of(
        &proposed,
        &draft_line,
        Some(&ref_line),
        Some(&pol),
        &tiers,
        &u,
    )
    .expect("tier proposal succeeds");

    assert_eq!(
        tier_lines[1].buy,
        Some(rate("32.12503213")),
        "must use MidpointAwayFromZero (HALF_UP), not banker's rounding"
    );
}
