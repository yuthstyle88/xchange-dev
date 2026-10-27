//! XC-V9-PRICING PV-15 / PV-15b / PV-15c (pricing-tiers.md §15.7, B5; `H:` §6, §10.6; V9:pricing
//! `automaticBoardProjection`): whether the board may show a published line. Fail closed for FOLLOW — `STALE` first,
//! then `BLOCKED`, then `MISMATCH`; `MANUAL` is never hidden; `AUTO` is never hidden by this rule. `H:` is the owner's
//! handoff `PRICING-DESIGN.md` and `V9:` its source excerpt (both kept out of the repo). Written before the
//! implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit, delete or weaken; add
//! tests in a file of your own. Pure: no database, no clock.

use uuid::Uuid;
use xc_rate::domain::pricing::follow_plan::FollowResultStatus;
use xc_rate::domain::pricing::follow_visibility::{
    FollowVisibility, HiddenReason, VisibilityInput,
};
use xc_rate::domain::pricing::tier_set::PricingSource;

/// MAIN's FOLLOW set that published.
const MAIN_SET: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0a01);
/// A newer FOLLOW set at MAIN.
const MAIN_NEWER: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0a02);
/// Branch B's FOLLOW set adjusted from `MAIN_SET`.
const B_SET: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0b01);
/// A newer FOLLOW set at branch B.
const B_NEWER: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0b02);

/// A FOLLOW line at MAIN on its latest set, which MAIN's latest result published; the source is current.
fn at_main() -> VisibilityInput {
    VisibilityInput {
        line_source: PricingSource::Follow,
        line_tier_set_id: Some(MAIN_SET),
        line_base_tier_set_id: None,
        at_main: true,
        source_current: true,
        branch_latest_follow: Some(MAIN_SET),
        main_latest_follow: Some(MAIN_SET),
        main_result: Some((FollowResultStatus::Published, Some(MAIN_SET))),
    }
}

/// A FOLLOW line at a followed branch B: its latest set, adjusted from MAIN's latest set, which published.
fn at_branch() -> VisibilityInput {
    VisibilityInput {
        line_source: PricingSource::Follow,
        line_tier_set_id: Some(B_SET),
        line_base_tier_set_id: Some(MAIN_SET),
        at_main: false,
        source_current: true,
        branch_latest_follow: Some(B_SET),
        main_latest_follow: Some(MAIN_SET),
        main_result: Some((FollowResultStatus::Published, Some(MAIN_SET))),
    }
}

fn of(input: VisibilityInput) -> Option<HiddenReason> {
    FollowVisibility::of(&input)
}

/// PV-13 (§15.7): a current FOLLOW line on the set MAIN published is shown, at MAIN and at a followed branch.
#[test]
fn a_current_published_follow_line_is_shown() {
    assert_eq!(of(at_main()), None, "MAIN");
    assert_eq!(of(at_branch()), None, "branch B");
}

/// PV-15 (§15.7 "`MANUAL` is never hidden"; B5): a MANUAL line with a stale anchor source is shown, whatever MAIN's
/// FOLLOW result says.
#[test]
fn a_manual_line_is_never_hidden() {
    for at in [at_main(), at_branch()] {
        let manual = VisibilityInput {
            line_source: PricingSource::Manual,
            source_current: false,
            main_result: Some((FollowResultStatus::Blocked, None)),
            branch_latest_follow: None,
            ..at
        };
        assert_eq!(of(manual), None);
    }
}

/// PV-15 (§15.7 "`AUTO` is never hidden by this rule"): an AUTO line with a stale source and a blocked MAIN result is
/// shown (today's `Freshness` keeps the publication's `stale`).
#[test]
fn an_auto_line_is_never_hidden_by_this_rule() {
    let auto = VisibilityInput {
        line_source: PricingSource::Auto,
        line_tier_set_id: None,
        line_base_tier_set_id: None,
        source_current: false,
        branch_latest_follow: None,
        main_latest_follow: None,
        main_result: Some((FollowResultStatus::Blocked, None)),
        ..at_branch()
    };
    assert_eq!(of(auto), None);
}

/// PV-11 (§15.7 `STALE`; `H:` §10.6): a FOLLOW line whose anchor source is not current is hidden `STALE`, at MAIN and
/// at a followed branch.
#[test]
fn a_follow_line_on_a_stale_source_is_hidden_stale() {
    for at in [at_main(), at_branch()] {
        let stale = VisibilityInput {
            source_current: false,
            ..at
        };
        assert_eq!(of(stale), Some(HiddenReason::Stale));
    }
}

/// PV-11 (follow_visibility.rs: "the first that holds, in this order"): stale, blocked and mismatched at once ⇒
/// `STALE`.
#[test]
fn stale_comes_first() {
    let all_three = VisibilityInput {
        source_current: false,
        branch_latest_follow: Some(B_NEWER),
        main_result: Some((FollowResultStatus::Blocked, None)),
        ..at_branch()
    };
    assert_eq!(of(all_three), Some(HiddenReason::Stale));
}

/// PV-15b (§15.7 `BLOCKED`): branch B follows; MAIN's latest plan is BLOCKED ⇒ B's FOLLOW line is hidden `BLOCKED`.
#[test]
fn a_followed_branch_line_is_hidden_when_mains_plan_is_blocked() {
    let blocked = VisibilityInput {
        main_result: Some((FollowResultStatus::Blocked, None)),
        ..at_branch()
    };
    assert_eq!(of(blocked), Some(HiddenReason::Blocked));
}

/// PV-15b (§15.7): MAIN's own FOLLOW line is hidden `BLOCKED` too, and `BLOCKED` comes before `MISMATCH` (a blocked
/// result never published that set either).
#[test]
fn blocked_comes_before_mismatch() {
    let blocked = VisibilityInput {
        main_result: Some((FollowResultStatus::Blocked, None)),
        ..at_main()
    };
    assert_eq!(of(blocked), Some(HiddenReason::Blocked));
}

/// PV-15c (§15.7 `MISMATCH`: "MAIN's latest result is not `PUBLISHED` with that same set"): MAIN re-planned
/// different tiers and the publish failed. MAIN's line and B's line are both hidden `MISMATCH`.
// reading: a failed publish leaves MAIN's latest result `CANDIDATE` (with no set, or with a set nothing published);
// either way it is not `PUBLISHED` with the line's set.
#[test]
fn a_replan_that_did_not_publish_hides_main_and_the_branch_as_mismatch() {
    for result in [
        (FollowResultStatus::Candidate, None),
        (FollowResultStatus::Candidate, Some(MAIN_NEWER)),
    ] {
        let main = VisibilityInput {
            main_result: Some(result),
            ..at_main()
        };
        assert_eq!(of(main), Some(HiddenReason::Mismatch), "MAIN {result:?}");
        let branch = VisibilityInput {
            main_result: Some(result),
            ..at_branch()
        };
        assert_eq!(
            of(branch),
            Some(HiddenReason::Mismatch),
            "branch B {result:?}"
        );
    }
}

/// PV-15c (§15.7): MAIN's latest result published another set ⇒ the line on the older set is `MISMATCH`.
#[test]
fn a_result_published_with_another_set_is_mismatch() {
    let main = VisibilityInput {
        main_result: Some((FollowResultStatus::Published, Some(MAIN_NEWER))),
        main_latest_follow: Some(MAIN_NEWER),
        branch_latest_follow: Some(MAIN_NEWER),
        ..at_main()
    };
    assert_eq!(of(main), Some(HiddenReason::Mismatch));
}

/// PV-15c (§15.7 `MISMATCH`: "the line's `tier_set_id` is not the branch's latest FOLLOW tier set"): B wrote a newer
/// FOLLOW set that its publication does not carry.
#[test]
fn a_line_not_on_the_branchs_latest_follow_set_is_mismatch() {
    let branch = VisibilityInput {
        branch_latest_follow: Some(B_NEWER),
        ..at_branch()
    };
    assert_eq!(of(branch), Some(HiddenReason::Mismatch));
    let main = VisibilityInput {
        branch_latest_follow: Some(MAIN_NEWER),
        ..at_main()
    };
    assert_eq!(of(main), Some(HiddenReason::Mismatch), "MAIN");
}

/// PV-15c (§15.7 `MISMATCH`: "at a followed branch, the set's `base_tier_set_id` is not MAIN's latest FOLLOW set"):
/// MAIN has a newer FOLLOW set that B's set was not adjusted from.
#[test]
fn a_branch_set_not_based_on_mains_latest_follow_set_is_mismatch() {
    let branch = VisibilityInput {
        main_latest_follow: Some(MAIN_NEWER),
        main_result: Some((FollowResultStatus::Published, Some(MAIN_NEWER))),
        ..at_branch()
    };
    assert_eq!(of(branch), Some(HiddenReason::Mismatch));
    let no_base = VisibilityInput {
        line_base_tier_set_id: None,
        ..at_branch()
    };
    assert_eq!(
        of(no_base),
        Some(HiddenReason::Mismatch),
        "a followed branch's set with no base"
    );
}

/// PV-11 (§15.7 fail closed): a FOLLOW line with no MAIN result at all is not shown as published.
// reading: no `pricing_follow_results` row means nothing says MAIN published that set, so `MISMATCH`.
#[test]
fn a_follow_line_with_no_main_result_is_mismatch() {
    for at in [at_main(), at_branch()] {
        let none = VisibilityInput {
            main_result: None,
            ..at
        };
        assert_eq!(of(none), Some(HiddenReason::Mismatch));
    }
}
