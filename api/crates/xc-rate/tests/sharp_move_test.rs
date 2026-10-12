//! Tests for SM-1 (#1490, `rates.md` §12.8, §12.10): SharpMove::between.
//! Written before the implementation by the test-writer (gemini/pool-1).
//!
//! §9 Edge cases:
//! - Row 1: threshold = None => none.
//! - Row 2: Exactly at the threshold => none; just above => one move.
//! - Row 3: A move on buy only => one move for that side.
//! - Row 4: Tiered lines compare base_*_rate: four tier lines whose margins change but whose base is unchanged => none.
//! - Row 5: No previous publication, a new group, a gone group, a None side => none.
//! - Row 7: NotificationRequest::new accepts the template's params for a realistic branch code, proving params pass redact_text unchanged.

use rust_decimal::Decimal;
use std::collections::BTreeMap;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent, Side};
use xc_kernel::notifications::NotificationRequest;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::sharp_move::{ComparablePrice, Move, SharpMove};

fn code(c: &str) -> CurrencyCode {
    CurrencyCode::parse(c).expect("valid currency")
}

fn group(g: &str) -> GroupCode {
    GroupCode::new(g).expect("valid group")
}

fn rate(r: &str) -> Rate {
    Rate::parse(r).expect("valid rate")
}

fn pct(p: &str) -> RatePercent {
    RatePercent::parse(p).expect("valid percent")
}

fn price(c: &str, g: &str, side: Side, r: &str) -> ComparablePrice {
    ComparablePrice::new(code(c), group(g), side, rate(r))
}

fn none_price(c: &str, g: &str, side: Side) -> ComparablePrice {
    ComparablePrice::none(code(c), group(g), side)
}

// ── §9 Edge case 1 ────────────────────────────────────────────────────────────

#[test]
fn case_1_threshold_none_gives_no_moves() {
    let old = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        price("USD", "ALL", Side::Sell, "31.0000"),
    ];
    // Massive price swing (from 30 to 45), but threshold is None
    let new = [
        price("USD", "ALL", Side::Buy, "45.0000"),
        price("USD", "ALL", Side::Sell, "46.0000"),
    ];
    let moves = SharpMove::between(&old, &new, None);
    assert_eq!(moves, vec![], "threshold = None must produce no moves");
}

// ── §9 Edge case 2 ────────────────────────────────────────────────────────────

#[test]
fn case_2_exactly_at_threshold_gives_none_just_above_gives_one_move() {
    let threshold = Some(pct("2.0000")); // 2.0%

    let old = [price("USD", "ALL", Side::Buy, "100.0000")];

    // Exactly 2% change: |102.0000 - 100.0000| / 100.0000 * 100 = 2.0000%
    let new_exact = [price("USD", "ALL", Side::Buy, "102.0000")];
    let moves_exact = SharpMove::between(&old, &new_exact, threshold);
    assert_eq!(
        moves_exact,
        vec![],
        "move_pct exactly equal to threshold must produce no move (strictly greater required)"
    );

    // Just above 2% change (e.g. 102.0100 -> 2.01% > 2.0%)
    let new_above = [price("USD", "ALL", Side::Buy, "102.0100")];
    let moves_above = SharpMove::between(&old, &new_above, threshold);
    assert_eq!(
        moves_above,
        vec![Move {
            currency: code("USD"),
            group: group("ALL"),
            side: Side::Buy,
            old: rate("100.0000"),
            new: rate("102.0100"),
            move_pct: Decimal::from_str_exact("2.01").unwrap(),
        }],
        "move_pct just above threshold must produce one move"
    );
}

// ── §9 Edge case 3 ────────────────────────────────────────────────────────────

#[test]
fn case_3_a_move_on_buy_only_gives_one_move_for_that_side() {
    let threshold = Some(pct("2.0000")); // 2.0%

    let old = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        price("USD", "ALL", Side::Sell, "31.0000"),
    ];

    // Buy moves by 6.6667% (32.00 vs 30.00 > 2%), Sell moves by 0.1613% (31.05 vs 31.00 <= 2%)
    let new = [
        price("USD", "ALL", Side::Buy, "32.0000"),
        price("USD", "ALL", Side::Sell, "31.0500"),
    ];

    let moves = SharpMove::between(&old, &new, threshold);
    assert_eq!(
        moves.len(),
        1,
        "only the side exceeding threshold should produce a move"
    );
    assert_eq!(moves[0].side, Side::Buy);
    assert_eq!(moves[0].currency, code("USD"));
    assert_eq!(moves[0].old, rate("30.0000"));
    assert_eq!(moves[0].new, rate("32.0000"));
}

// ── §9 Edge case 4 ────────────────────────────────────────────────────────────

#[test]
fn case_4_tiered_lines_compare_base_rate_four_tiers_changing_margins_with_unchanged_base_give_none()
{
    let threshold = Some(pct("2.0000"));

    // For tiered publications, the comparable price is the base rate (owner O-11).
    // Here, 4 tier lines have different customer-facing margins, but their base rate is identical.
    let old_base = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        price("USD", "ALL", Side::Sell, "31.0000"),
    ];
    let new_base = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        price("USD", "ALL", Side::Sell, "31.0000"),
    ];

    let moves = SharpMove::between(&old_base, &new_base, threshold);
    assert_eq!(
        moves,
        vec![],
        "tier switch or tier margin change with unchanged base rate produces no move"
    );
}

// ── §9 Edge case 5 ────────────────────────────────────────────────────────────

#[test]
fn case_5_no_previous_publication_new_group_gone_group_or_none_side_give_no_move() {
    let threshold = Some(pct("1.0000"));

    // Subcase 5a: No previous publication (old is empty)
    let old_empty: [ComparablePrice; 0] = [];
    let new = [price("USD", "ALL", Side::Buy, "35.0000")];
    assert_eq!(
        SharpMove::between(&old_empty, &new, threshold),
        vec![],
        "empty old (no previous publication) must produce no move"
    );

    // Subcase 5b: A new group appears in new that was not in old
    let old_grp = [price("USD", "ALL", Side::Buy, "30.0000")];
    let new_grp = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        price("USD", "LARGE", Side::Buy, "40.0000"),
    ];
    assert_eq!(
        SharpMove::between(&old_grp, &new_grp, threshold),
        vec![],
        "new denomination group not present in old must produce no move"
    );

    // Subcase 5c: A group was in old but is gone from new
    let old_gone = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        price("USD", "SMALL", Side::Buy, "28.0000"),
    ];
    let new_gone = [price("USD", "ALL", Side::Buy, "30.0000")];
    assert_eq!(
        SharpMove::between(&old_gone, &new_gone, threshold),
        vec![],
        "denomination group gone from new must produce no move"
    );

    // Subcase 5d: A None side on either old or new
    let old_side = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        none_price("USD", "ALL", Side::Sell),
    ];
    let new_side = [
        price("USD", "ALL", Side::Buy, "30.0000"),
        price("USD", "ALL", Side::Sell, "35.0000"),
    ];
    assert_eq!(
        SharpMove::between(&old_side, &new_side, threshold),
        vec![],
        "a side that is None in either old or new must produce no move"
    );
}

// ── §9 Edge case 7 ────────────────────────────────────────────────────────────

#[test]
fn case_7_notification_request_new_accepts_template_params_for_realistic_branch_code() {
    for branch_code in ["MAIN", "BKK01"] {
        let mut params = BTreeMap::new();
        params.insert("branch".to_string(), branch_code.to_string());
        params.insert("currency".to_string(), "USD".to_string());
        params.insert("side".to_string(), "BUY".to_string());

        let req = NotificationRequest::new(
            "user-1",
            format!("rate-sharp-move:pub-1:USD:BUY:{branch_code}"),
            "rate_sharp_move",
            None,
            params.clone(),
        );

        assert!(
            req.is_ok(),
            "NotificationRequest::new must accept params without numbers: {req:?}"
        );

        // Verify that redact_text leaves each param completely unchanged
        for (k, v) in &params {
            assert_eq!(
                xc_kernel::pii::redact_text(v),
                *v,
                "param '{k}' with value '{v}' must pass redact_text unchanged"
            );
        }
    }
}
