//! Up-front tests for AP-1 (design `docs/design/blueprint/user-management-rbac.md` §11.9): `ApprovalDecider`.
//! Written before the implementation by the BE lane controller (AP-0 skeleton, `feat/approval-perms-skeleton`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9:
//! - `holds_anywhere(who, p)` is true iff some row `r` of `who` has `RoleGrants::holds(r.group_code(), p)`;
//! - `holds_on(who, p, branch)` is the same, and also requires `r.covers(branch)` **on that same row**;
//! - the base role (`r.role`) is not read.
//!
//! No database. `RoleGrants::install` is process-global, so every test here holds `LOCK` and puts the seeded
//! snapshot back before it ends. Every installed snapshot keeps the nine built-in groups (CI runs each test in its
//! own process under nextest; a plain `cargo test` shares the binary with other files that install snapshots too).

use std::sync::Mutex;

use chrono::{Duration, Utc};
use xc_auth::domain::access::permission_module::APPROVAL_KINDS;
use xc_auth::domain::access::{
    Actor, ApprovalDecider, Group, Permission, Role, RoleGrants, RoleGrantsSnapshot, ScopedRole,
};
use xc_kernel::ids::BranchId;

use Permission as P;

static LOCK: Mutex<()> = Mutex::new(());

fn locked() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Puts the seeded snapshot back when a test passes. A failing test leaves it (installing while unwinding would
/// abort the binary); every test here installs what it needs first.
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            RoleGrants::install(RoleGrantsSnapshot::seeded());
        }
    }
}

const ORG: &str = "org-1";

fn custom(code: &str, base: Role) -> Group {
    Group {
        code: code.to_owned(),
        name: code.to_owned(),
        base,
        built_in: false,
    }
}

/// The nine built-in groups at their seed, with `edit` applied to each set, plus `extra` groups.
fn install(edit: impl Fn(Role, &mut Vec<Permission>), extra: Vec<(Group, Vec<Permission>)>) {
    let mut groups: Vec<(Group, Vec<Permission>)> = RoleGrants::BUILT_IN
        .iter()
        .map(|&r| {
            let mut set = RoleGrants::seed(r);
            edit(r, &mut set);
            (Group::built_in(r), set)
        })
        .collect();
    groups.extend(extra);
    RoleGrants::install(RoleGrantsSnapshot::new(40, groups));
}

fn seeded_plus(extra: Vec<(Group, Vec<Permission>)>) {
    install(|_, _| {}, extra);
}

/// A row of group `code` (base `base`) at `branch` (`None` = organisation-wide), valid now.
fn row(code: &str, base: Role, branch: Option<&str>) -> ScopedRole {
    let now = Utc::now();
    ScopedRole::in_group(
        code,
        base,
        branch.map(BranchId::new),
        now - Duration::hours(1),
        None,
    )
}

fn actor(rows: Vec<ScopedRole>) -> Actor {
    Actor::new("u-decider", ORG, rows, None)
}

/// `base`'s seed with `add` added (and `ApprovalsDecide`, which follows the kinds) and `remove` taken out.
fn set_of(base: Role, add: &[Permission], remove: &[Permission]) -> Vec<Permission> {
    let mut set: Vec<Permission> = RoleGrants::seed(base)
        .into_iter()
        .filter(|p| !remove.contains(p))
        .collect();
    for p in add {
        if !set.contains(p) {
            set.push(*p);
        }
    }
    if set.iter().any(|p| APPROVAL_KINDS.contains(p)) && !set.contains(&P::ApprovalsDecide) {
        set.push(P::ApprovalsDecide);
    }
    if !set.iter().any(|p| APPROVAL_KINDS.contains(p)) {
        set.retain(|p| *p != P::ApprovalsDecide);
    }
    set
}

/// §11.9 A2 (decider side): custom group `NIGHT_FIN`, base TELLER, granted `ApproveTransactionReversal`, row at B1.
/// It decides a B1 reversal and not a B2 one. Its base (TELLER) holds no approval kind, so a body that reads the
/// base role answers false here.
#[test]
fn a2_a_teller_based_group_granted_reversal_decides_on_its_branch_only() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("NIGHT_FIN", Role::Teller),
        set_of(Role::Teller, &[P::ApproveTransactionReversal], &[]),
    )]);
    let who = actor(vec![row("NIGHT_FIN", Role::Teller, Some("B1"))]);

    assert!(ApprovalDecider::holds_on(
        &who,
        P::ApproveTransactionReversal,
        Some("B1")
    ));
    assert!(
        !ApprovalDecider::holds_on(&who, P::ApproveTransactionReversal, Some("B2")),
        "a B1 row does not cover B2"
    );
    assert!(ApprovalDecider::holds_anywhere(
        &who,
        P::ApproveTransactionReversal
    ));
    assert!(
        !ApprovalDecider::holds_anywhere(&who, P::ApproveCountDifference),
        "NIGHT_FIN was granted the reversal kind only"
    );
}

/// §11.9 A3 (decider side): built-in FINANCE with `ApproveCountDifference` removed. A FINANCE@B1 user no longer
/// holds it at B1, though its base is FINANCE; it still holds the reversal kind it was not stripped of.
#[test]
fn a3_finance_with_count_difference_removed_no_longer_holds_it() {
    let _g = locked();
    let _r = Restore;
    install(
        |r, set| {
            if r == Role::Finance {
                set.retain(|p| *p != P::ApproveCountDifference);
            }
        },
        vec![],
    );
    let who = actor(vec![row("FINANCE", Role::Finance, Some("B1"))]);

    assert!(!ApprovalDecider::holds_on(
        &who,
        P::ApproveCountDifference,
        Some("B1")
    ));
    assert!(!ApprovalDecider::holds_anywhere(
        &who,
        P::ApproveCountDifference
    ));
    assert!(
        ApprovalDecider::holds_on(&who, P::ApproveTransactionReversal, Some("B1")),
        "FINANCE still holds the reversal kind"
    );
}

/// §11.9 A6 (decider side): EXECUTIVE@B1 (branch-scoped) holds the four organisation kinds anywhere, which is what
/// the organisation policies ask. `holds_on` still answers by the row's branch.
#[test]
fn a6_a_branch_scoped_executive_holds_the_organisation_kinds_anywhere() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let who = actor(vec![row("EXECUTIVE", Role::Executive, Some("B1"))]);

    for p in [
        P::ApproveCampaignVersion,
        P::ApproveRatePublication,
        P::ApproveLoyaltyRuleVersion,
        P::ApproveCashTransferAboveLimit,
    ] {
        assert!(ApprovalDecider::holds_anywhere(&who, p), "{p:?}");
    }
    assert!(
        !ApprovalDecider::holds_on(&who, P::ApproveCampaignVersion, Some("B2")),
        "holds_on requires the same row to cover B2"
    );
    assert!(
        !ApprovalDecider::holds_anywhere(&who, P::ApproveExchangeTransaction),
        "the seed gives EXECUTIVE no branch kind"
    );
}

/// An organisation-wide row (`branch = None`) covers any branch, and a request with no branch.
#[test]
fn an_organisation_wide_row_covers_any_branch() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let who = actor(vec![row("FINANCE", Role::Finance, None)]);

    for branch in [Some("B1"), Some("B9"), None] {
        assert!(
            ApprovalDecider::holds_on(&who, P::ApproveTransactionReversal, branch),
            "an org-wide FINANCE row covers {branch:?}"
        );
    }
}

/// A branch row does not cover a request with no branch (`ScopedRole::covers(None)` is false for it).
#[test]
fn a_branch_row_does_not_cover_a_request_with_no_branch() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let who = actor(vec![row("BRANCH_MANAGER", Role::BranchManager, Some("B1"))]);

    assert!(ApprovalDecider::holds_on(
        &who,
        P::ApproveNegativeStock,
        Some("B1")
    ));
    assert!(!ApprovalDecider::holds_on(
        &who,
        P::ApproveNegativeStock,
        None
    ));
}

/// §11.9 "the base role is not read": a group with base FINANCE whose set lacks the reversal kind does not hold
/// it, though built-in FINANCE does.
#[test]
fn the_base_role_is_not_read() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("FIN_LITE", Role::Finance),
        set_of(
            Role::Finance,
            &[],
            &[P::ApproveTransactionReversal, P::ApproveCountDifference],
        ),
    )]);
    let lite = actor(vec![row("FIN_LITE", Role::Finance, Some("B1"))]);
    let fin = actor(vec![row("FINANCE", Role::Finance, Some("B1"))]);

    assert!(!ApprovalDecider::holds_on(
        &lite,
        P::ApproveTransactionReversal,
        Some("B1")
    ));
    assert!(!ApprovalDecider::holds_anywhere(
        &lite,
        P::ApproveTransactionReversal
    ));
    assert!(ApprovalDecider::holds_on(
        &fin,
        P::ApproveTransactionReversal,
        Some("B1")
    ));
}

/// `holds_on` needs one row that both holds the permission and covers the branch. A row that holds it at B2 and a
/// row that covers B1 but does not hold it do not add up to B1.
#[test]
fn holds_on_needs_one_row_that_both_holds_and_covers() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let who = actor(vec![
        row("TELLER", Role::Teller, Some("B1")),
        row("BRANCH_MANAGER", Role::BranchManager, Some("B2")),
    ]);

    assert!(
        !ApprovalDecider::holds_on(&who, P::ApproveExchangeTransaction, Some("B1")),
        "the B1 row (TELLER) does not hold it; the B2 row does not cover B1"
    );
    assert!(ApprovalDecider::holds_on(
        &who,
        P::ApproveExchangeTransaction,
        Some("B2")
    ));
    assert!(ApprovalDecider::holds_anywhere(
        &who,
        P::ApproveExchangeTransaction
    ));
}

/// No rows, or only rows of a code the snapshot does not hold, hold nothing.
#[test]
fn no_rows_or_an_unknown_group_hold_nothing() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let nobody = actor(vec![]);
    let ghost = actor(vec![row("NO_SUCH_GROUP", Role::Finance, None)]);

    for p in APPROVAL_KINDS {
        assert!(!ApprovalDecider::holds_anywhere(&nobody, p), "{p:?}");
        assert!(!ApprovalDecider::holds_on(&nobody, p, Some("B1")), "{p:?}");
        assert!(
            !ApprovalDecider::holds_anywhere(&ghost, p),
            "{p:?}: an unknown code holds nothing, whatever its base"
        );
    }
}
