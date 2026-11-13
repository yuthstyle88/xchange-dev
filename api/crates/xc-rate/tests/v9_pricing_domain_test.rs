//! XC-V9-PRICING (pricing-tiers.md §15.2, §15.5, §15.7, §15.8a, §15.10): the wire strings of the V.9 pricing enums,
//! `MAIN_PUSH` reading as `MANUAL` (§15.7), `TierRate::to_check`, and `SuggestionSettings::min_spread_for` (gap 10).
//! `H:` is the owner's handoff `PRICING-DESIGN.md` (kept out of the repo). Written before the implementation by the
//! sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit, delete or weaken; add tests in a file of your
//! own. Pure: no database, no clock.

use rust_decimal::Decimal;
use xc_kernel::money::Rate;
use xc_rate::domain::pricing::branch_policy::PolicyFrom;
use xc_rate::domain::pricing::display_settings::DisplayFrom;
use xc_rate::domain::pricing::follow_plan::{FollowReason, FollowResultStatus};
use xc_rate::domain::pricing::follow_visibility::HiddenReason;
use xc_rate::domain::pricing::pricing_mode::PricingMode;
use xc_rate::domain::pricing::pricing_suggestion::TierPrice;
use xc_rate::domain::pricing::suggestion_settings::SuggestionSettings;
use xc_rate::domain::pricing::tier_set::{PricingSource, TierRate, TierSetSource};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("rate")
}

fn tier(buy: &str, sell: &str) -> TierRate {
    TierRate {
        buy: rate(buy),
        sell: rate(sell),
    }
}

/// PV-a (§15.2 `pricing_tier_sets.source`): `MANUAL` · `MAIN_PUSH` · `FOLLOW`, both ways.
#[test]
fn tier_set_sources_are_the_columns_check_values_both_ways() {
    for (source, text) in [
        (TierSetSource::Manual, "MANUAL"),
        (TierSetSource::MainPush, "MAIN_PUSH"),
        (TierSetSource::Follow, "FOLLOW"),
    ] {
        assert_eq!(source.as_str(), text);
        assert_eq!(TierSetSource::parse(text), Some(source), "{text}");
    }
    for unknown in ["AUTO", "", "MAINPUSH"] {
        assert_eq!(TierSetSource::parse(unknown), None, "{unknown:?}");
    }
}

/// PV-i (§15.7): `pricingSource` is `AUTO` · `MANUAL` · `FOLLOW`.
#[test]
fn pricing_sources_are_auto_manual_follow() {
    assert_eq!(PricingSource::Auto.as_str(), "AUTO");
    assert_eq!(PricingSource::Manual.as_str(), "MANUAL");
    assert_eq!(PricingSource::Follow.as_str(), "FOLLOW");
}

/// PV-i (§15.7): a `MAIN_PUSH` tier set reads as `MANUAL` on the board and the public projection; the other two keep
/// their name.
#[test]
fn a_main_push_reads_as_manual() {
    assert_eq!(
        TierSetSource::MainPush.pricing_source(),
        PricingSource::Manual
    );
    assert_eq!(
        TierSetSource::Manual.pricing_source(),
        PricingSource::Manual
    );
    assert_eq!(
        TierSetSource::Follow.pricing_source(),
        PricingSource::Follow
    );
}

/// PV-a (§15.8a `branchPolicy.from`): `"currency"` · `"*"` · `"default"`.
#[test]
fn policy_from_is_currency_star_or_default() {
    assert_eq!(PolicyFrom::Currency.as_str(), "currency");
    assert_eq!(PolicyFrom::Branch.as_str(), "*");
    assert_eq!(PolicyFrom::Default.as_str(), "default");
}

/// PV-a (§15.8a `display.from`): `"branch"` · `"MAIN"` · `"default"`.
#[test]
fn display_from_is_branch_main_or_default() {
    assert_eq!(DisplayFrom::Branch.as_str(), "branch");
    assert_eq!(DisplayFrom::Main.as_str(), "MAIN");
    assert_eq!(DisplayFrom::Default.as_str(), "default");
}

/// PV-f (§15.5 `pricing_follow_results.reasons`) (`H:` §6's names).
#[test]
fn follow_reasons_are_v9s_flag_names() {
    for (reason, text) in [
        (FollowReason::NoAnchor, "NO_ANCHOR"),
        (FollowReason::StaleSource, "STALE_SOURCE"),
        (FollowReason::Precision, "PRECISION"),
        (FollowReason::BelowCost, "BELOW_COST"),
        (FollowReason::LargeMove, "LARGE_MOVE"),
        (FollowReason::InvalidTiers, "INVALID_TIERS"),
        (FollowReason::RuleNotSet, "RULE_NOT_SET"),
    ] {
        assert_eq!(reason.as_str(), text);
        assert_eq!(FollowReason::parse(text), Some(reason));
    }
}

/// PV-f (§15.2 `pricing_follow_results.status`): `CANDIDATE` · `BLOCKED` · `PUBLISHED`, both ways.
#[test]
fn follow_result_statuses_are_the_columns_check_values_both_ways() {
    for (status, text) in [
        (FollowResultStatus::Candidate, "CANDIDATE"),
        (FollowResultStatus::Blocked, "BLOCKED"),
        (FollowResultStatus::Published, "PUBLISHED"),
    ] {
        assert_eq!(status.as_str(), text);
        assert_eq!(FollowResultStatus::parse(text), Some(status), "{text}");
    }
    for unknown in ["STALE", "", "DONE"] {
        assert_eq!(FollowResultStatus::parse(unknown), None, "{unknown:?}");
    }
}

/// PV-i (§15.7 `hiddenReason`): `STALE` · `BLOCKED` · `MISMATCH`.
#[test]
fn hidden_reasons_are_stale_blocked_mismatch() {
    assert_eq!(HiddenReason::Stale.as_str(), "STALE");
    assert_eq!(HiddenReason::Blocked.as_str(), "BLOCKED");
    assert_eq!(HiddenReason::Mismatch.as_str(), "MISMATCH");
}

/// PV-f (§15.2 `pricing_modes.mode`): `FOLLOW` · `MANUAL`, both ways; `AUTO` is no row, never a stored mode (§15.1).
#[test]
fn pricing_modes_are_follow_or_manual_both_ways() {
    for (mode, text) in [
        (PricingMode::Follow, "FOLLOW"),
        (PricingMode::Manual, "MANUAL"),
    ] {
        assert_eq!(mode.as_str(), text);
        assert_eq!(PricingMode::parse(text), Some(mode), "{text}");
    }
    for unknown in ["AUTO", "MAIN_PUSH", ""] {
        assert_eq!(PricingMode::parse(unknown), None, "{unknown:?}");
    }
}

/// PV-2 (§15.3 step 2): an approved set reaches `TierCheck::validate` as tiers 1..=4 with both sides present, the
/// same prices, in the same order. PV-1's tiers (`H:` §4.4).
#[test]
fn an_approved_tier_set_is_checked_as_tiers_one_to_four() {
    let usd = [
        tier("33.23", "33.87"),
        tier("33.24", "33.86"),
        tier("33.26", "33.84"),
        tier("33.29", "33.80"),
    ];
    let expected = [
        ("33.23", "33.87"),
        ("33.24", "33.86"),
        ("33.26", "33.84"),
        ("33.29", "33.80"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (buy, sell))| TierPrice {
        tier_no: (i + 1) as u8,
        buy: Some(rate(buy)),
        sell: Some(rate(sell)),
    })
    .collect::<Vec<_>>();
    assert_eq!(TierRate::to_check(&usd).to_vec(), expected);
}

/// PV-l, gap 10 (§15.10; `H:` §4.1: defaults → branch `*` → currency): the currency override wins, then the branch `'*'`
/// row's `min_spread_pct`, then V.9's 0.3.
#[test]
fn min_spread_is_the_currency_override_then_the_branch_then_v9s() {
    assert_eq!(
        SuggestionSettings::min_spread_for(Some(d("0.5")), Some(d("0.4"))),
        d("0.5")
    );
    assert_eq!(
        SuggestionSettings::min_spread_for(None, Some(d("0.4"))),
        d("0.4")
    );
    assert_eq!(SuggestionSettings::min_spread_for(None, None), d("0.3"));
    // reading: 0 is a set value (the column's CHECK allows 0..5), not "unset".
    assert_eq!(
        SuggestionSettings::min_spread_for(Some(d("0")), Some(d("0.4"))),
        d("0")
    );
    assert_eq!(
        SuggestionSettings::min_spread_for(None, Some(d("0"))),
        d("0")
    );
}
