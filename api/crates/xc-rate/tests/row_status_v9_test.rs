//! G8 — up-front tests: a pricing board row's status follows V.9's `statusOf` (V.9 source
//! `xchange-desk-v9-20261015/src/devices-public/pricing.js:84-94`, the owner's S-V9 on #5055; `pricing-tiers.md` §5.1;
//! `docs/design/v9-v3-comparison-mapping.md` CMP-37, CMP-46; G8 on #5055).
//!
//! Written before the implementation by the ticket author (claude/bc8fd29a, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own. Pure: no database, no clock.
//!
//! The defect at origin/main `2bd6d41ab`: `RowStatus::of(&Suggestion, approved, enabled)` is a skeleton
//! (`row_status.rs:3-4`), and its only caller passes `enabled` for `approved` (`pricing_board.rs:570`). So no row ever
//! reads `Unpriced` or `Tiers`, and an enabled row reads `Current` whatever its prices are.
//!
//! The seam: `RowStatus::of_row(&RowFacts) -> RowStatus`, with
//! `RowFacts { suggestion: &Suggestion, priceable: bool, published: Option<&[TierPrice]> }`:
//! - `priceable` is V.9's: the code is a currency v3 can store (`CurrencyCode::parse` succeeds), **not** "enabled at the
//!   branch". V.9's `enabled` never enters `statusOf`.
//! - `published` is the branch's published tiers for the currency. In v3 an approved price is a published one (P-3), so
//!   V.9's `approved` and `shop` (tier 1 pushed to the counter) are both read from it.
//!
//! The order is V.9's, first match wins:
//!
//! | # | Case | Status |
//! |---|---|---|
//! | S1 | not priceable | `Reference` |
//! | S2 | no suggested tiers | `Manual` |
//! | S3 | a blocking flag | `Review` |
//! | S4 | published tiers equal the suggested 4 tiers | `Current` |
//! | S5 | nothing published, or published tier 1 has no buy above 0 | `Unpriced` |
//! | S6 | published tier 1 equals the suggested tier 1 (the 4-tier set differs) | `Tiers` |
//! | S7 | otherwise | `Pending` |
//!
//! V.9's base-following statuses (`based`, `basetodo`, `nobase`: a branch currency that follows MAIN) are not in this
//! ticket; `RowStatus` gains no variant here.

use xc_kernel::money::Rate;
use xc_rate::domain::pricing::pricing_flag::PricingFlag;
use xc_rate::domain::pricing::pricing_suggestion::{Suggestion, TierPrice};
use xc_rate::domain::pricing::row_status::{RowFacts, RowStatus};

fn rate(s: &str) -> Option<Rate> {
    Some(Rate::parse(s).expect("a rate"))
}

/// Four tiers whose tier 1 is (`b1`, `s1`) and whose tiers 2–4 step 0.01 apart, plus `shift` on tier 2's buy.
fn tiers(b1: &str, s1: &str, shift: &str) -> [TierPrice; 4] {
    let b: rust_decimal::Decimal = b1.parse().unwrap();
    let s: rust_decimal::Decimal = s1.parse().unwrap();
    let step: rust_decimal::Decimal = "0.01".parse().unwrap();
    let extra: rust_decimal::Decimal = shift.parse().unwrap();
    std::array::from_fn(|i| {
        let n = rust_decimal::Decimal::from(i as u32);
        let mut buy = b + step * n;
        if i == 1 {
            buy += extra;
        }
        TierPrice {
            tier_no: (i + 1) as u8,
            buy: rate(&buy.to_string()),
            sell: rate(&(s - step * n).to_string()),
        }
    })
}

fn suggested(flags: Vec<PricingFlag>) -> Suggestion {
    Suggestion {
        tiers: Some(tiers("32.40", "32.90", "0")),
        margins: None,
        flags,
    }
}

fn status(s: &Suggestion, priceable: bool, published: Option<&[TierPrice]>) -> RowStatus {
    RowStatus::of_row(&RowFacts {
        suggestion: s,
        priceable,
        published,
    })
}

#[test]
fn s1_a_currency_v3_cannot_store_is_reference_whatever_else_holds() {
    let s = suggested(vec![]);
    let same = tiers("32.40", "32.90", "0");
    assert_eq!(status(&s, false, Some(&same)), RowStatus::Reference);
    let none = Suggestion {
        tiers: None,
        margins: None,
        flags: vec![PricingFlag::NoAnchor],
    };
    assert_eq!(
        status(&none, false, None),
        RowStatus::Reference,
        "V.9 checks priceable before the anchor"
    );
}

#[test]
fn s2_no_suggested_tiers_is_manual() {
    let s = Suggestion {
        tiers: None,
        margins: None,
        flags: vec![PricingFlag::NoAnchor],
    };
    assert_eq!(status(&s, true, None), RowStatus::Manual);
    let published = tiers("32.40", "32.90", "0");
    assert_eq!(status(&s, true, Some(&published)), RowStatus::Manual);
}

#[test]
fn s3_a_blocking_flag_is_review_even_when_prices_match() {
    for flag in [
        PricingFlag::BelowCost,
        PricingFlag::StaleSource,
        PricingFlag::Precision,
    ] {
        let s = suggested(vec![flag]);
        let same = tiers("32.40", "32.90", "0");
        assert_eq!(
            status(&s, true, Some(&same)),
            RowStatus::Review,
            "{flag:?} blocks"
        );
    }
}

#[test]
fn s3b_a_non_blocking_flag_does_not_make_review() {
    let s = suggested(vec![PricingFlag::LargeMove, PricingFlag::NoTtQuote]);
    let same = tiers("32.40", "32.90", "0");
    assert_eq!(status(&s, true, Some(&same)), RowStatus::Current);
}

#[test]
fn s4_published_tiers_equal_to_the_suggestion_are_current() {
    let s = suggested(vec![]);
    let same = tiers("32.40", "32.90", "0");
    assert_eq!(status(&s, true, Some(&same)), RowStatus::Current);
}

#[test]
fn s5_nothing_published_or_no_tier1_buy_is_unpriced() {
    let s = suggested(vec![]);
    assert_eq!(status(&s, true, None), RowStatus::Unpriced);
    let mut no_buy = tiers("32.40", "32.90", "0");
    no_buy[0].buy = None;
    assert_eq!(status(&s, true, Some(&no_buy)), RowStatus::Unpriced);
    let empty: [TierPrice; 0] = [];
    assert_eq!(status(&s, true, Some(&empty)), RowStatus::Unpriced);
}

#[test]
fn s6_tier1_already_at_the_suggestion_but_the_set_differs_is_tiers() {
    let s = suggested(vec![]);
    let tier2_off = tiers("32.40", "32.90", "0.05");
    assert_eq!(status(&s, true, Some(&tier2_off)), RowStatus::Tiers);
}

#[test]
fn s7_tier1_away_from_the_suggestion_is_pending() {
    let s = suggested(vec![]);
    let other = tiers("32.30", "33.00", "0");
    assert_eq!(status(&s, true, Some(&other)), RowStatus::Pending);
    let only_sell_differs = tiers("32.40", "32.95", "0");
    assert_eq!(
        status(&s, true, Some(&only_sell_differs)),
        RowStatus::Pending
    );
}
