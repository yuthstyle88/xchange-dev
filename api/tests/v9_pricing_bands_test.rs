//! XC-V9-PRICING PV-21 (pricing-tiers.md §15.3 step 6, §15.6, §15.10 "Band boundaries end to end"; `H:` §4.1,
//! §10.2): a published MANUAL 4-tier set carries V.9's fixed `tierFor` bands — ≤ 10,000 / ≤ 50,000 / ≤ 200,000 /
//! above, THB, lower bound exclusive — whatever the branch policy's own bands, and the THB amounts 10,000 · 10,000.01 ·
//! 50,000 · 50,000.01 · 200,000 · 200,000.01 fall in tiers 1 · 2 · 2 · 3 · 3 · 4, BUY and SELL. The domain twin is
//! `xc-rate/tests/value_tier_test.rs:76-94`; this one goes through the approve and the published lines.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! §15.6 says the tier a customer's amount falls in "is still recorded for reference" (`H:` §8 `orderReference`).
//! Nothing records it today: `quote_lines` has no tier column (`20261014100000_quotes.sql`) and the quote line points
//! at tier 1 (PV-6). So this test pins the band bounds on the published tier lines and the tier those bounds give
//! (through `TierTable::pick`, the reader `line_for` uses), not a recorded value.
//!
//! Readings:
//! - reading: the tier lines are coded `T1`..`T4` (the codes `pricing_board.rs:256` reads as tier numbers).
//! - reading: "BUY and SELL" (`H:` §10.2) is the same THB band on each side; per side the test also checks that the
//!   band's line carries that tier's price on that side.

mod v9_approve_common;

use v9_approve_common::*;
use xc_kernel::money::Money;
use xc_rate::domain::publication::value_tier::{TierBand, TierCode, TierTable};

fn money(d: Option<rust_decimal::Decimal>) -> Option<Money> {
    d.map(|v| Money::parse(&v.to_string()).unwrap_or_else(|e| panic!("{v} is THB: {e:?}")))
}

/// PV-21 (`H:` §4.1, §10.2): the published bands are V.9's fixed THB bounds, not the policy's 20,000 / 100,000 /
/// 500,000, and the boundary amounts fall in tiers 1·2·2·3·3·4 on both sides.
#[tokio::test]
async fn v9_bands_the_boundary_amounts_fall_in_tiers_1_2_2_3_3_4() {
    // MAIN's policy has value tiers ON with its own bands 20,000 / 100,000 / 500,000 (see `Room::policy`): a MANUAL
    // currency must not use them (§15.3: `rate_policy_value_tiers` stay for AUTO currencies only).
    let room = main_room().await;
    approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("PV-2 approve");

    let lines = room.published(MAIN, "USD").await;
    let bounds: Vec<_> = lines
        .iter()
        .map(|l| {
            (
                l.value_tier.clone(),
                l.lower_exclusive_thb,
                l.upper_inclusive_thb,
            )
        })
        .collect();
    assert_eq!(
        bounds,
        vec![
            ("T1".to_string(), None, Some(dec("10000.00"))),
            (
                "T2".to_string(),
                Some(dec("10000.00")),
                Some(dec("50000.00"))
            ),
            (
                "T3".to_string(),
                Some(dec("50000.00")),
                Some(dec("200000.00"))
            ),
            ("T4".to_string(), Some(dec("200000.00")), None),
        ],
        "§15.3 step 6: V.9's tierFor bounds (H: §4.1)"
    );

    let table = TierTable::new(
        lines
            .iter()
            .map(|l| TierBand {
                code: TierCode::new(l.value_tier.clone()).expect("a tier code"),
                lower_exclusive_thb: money(l.lower_exclusive_thb),
                upper_inclusive_thb: money(l.upper_inclusive_thb),
            })
            .collect(),
    );
    let tier_of = |code: &str| -> usize {
        lines
            .iter()
            .position(|l| l.value_tier == code)
            .unwrap_or_else(|| panic!("{code} is a published line"))
    };

    for side in ["BUY", "SELL"] {
        for (thb, tier_no) in [
            ("10000.00", 1usize),
            ("10000.01", 2),
            ("50000.00", 2),
            ("50000.01", 3),
            ("200000.00", 3),
            ("200000.01", 4),
        ] {
            let picked = table
                .pick(Money::parse(thb).expect("THB"))
                .unwrap_or_else(|| panic!("{side} {thb}: one band holds it"));
            let i = tier_of(picked.as_str());
            assert_eq!(i + 1, tier_no, "{side} {thb} THB falls in tier {tier_no}");
            let (buy, sell) = PV1_TIERS[tier_no - 1];
            let (price, expected) = match side {
                "BUY" => (lines[i].buy, dec(buy)),
                _ => (lines[i].sell, dec(sell)),
            };
            assert_eq!(
                price,
                Some(expected),
                "{side} {thb}: tier {tier_no}'s {side} price"
            );
        }
    }
    room.finish().await;
}
