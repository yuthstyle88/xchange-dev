//! Up-front tests for AP-4 (design `docs/design/blueprint/user-management-rbac.md` §11.9): `CASH_TRANSFER`'s
//! `TransferApprovalPolicy::may_decide` decides by `ApproveCashTransferWithinLimit` / `ApproveCashTransferAboveLimit`
//! through `ApprovalDecider`, not by the row's base role.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9 for this kind:
//! - within the limit: `holds_on(who, ApproveCashTransferWithinLimit, from_branch)`, else `WITHIN_LIMIT`;
//! - above the limit (or no `within_limit`): `holds_on(who, ApproveCashTransferAboveLimit, from_branch)`, else
//!   `ABOVE_LIMIT` — a branch kind, because `transfer_flow.rs` pre-checks the source branch (must-fix 1);
//! - the refusal codes stay `ABOVE_LIMIT` / `WITHIN_LIMIT`, never `outOfScope`.
//!
//! The policy is asked directly (`or71_transfer_policy_test.rs` pins the seed's outcomes; they stay green).
//! Tests marked "passes today" lock the seed's outcome (A1/A4); the rest fail until AP-4 and AP-1 land.
//!
//! No database. `RoleGrants::install` is process-global: every test here holds `LOCK` and puts the seeded snapshot
//! back when it passes. CI runs nextest, one process per test, so other files' snapshots never meet these; under a
//! plain `cargo test` the package's files share one binary and may interleave.

use std::sync::Mutex;

use chrono::{Duration, Utc};
use serde_json::json;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::access::permission_module::APPROVAL_KINDS;
use xc_auth::domain::access::{
    Actor, Group, Permission, Role, RoleGrants, RoleGrantsSnapshot, ScopedRole,
};
use xc_cash::application::approval::TransferApprovalPolicy;
use xc_kernel::ids::BranchId;

use Permission as P;

static LOCK: Mutex<()> = Mutex::new(());

fn locked() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Puts the seeded snapshot back when a test passes (installing while unwinding would abort the binary).
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            RoleGrants::install(RoleGrantsSnapshot::seeded());
        }
    }
}

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn custom(code: &str, base: Role) -> Group {
    Group {
        code: code.to_owned(),
        name: code.to_owned(),
        base,
        built_in: false,
    }
}

/// The nine built-in groups at their seed with `edit` applied, plus `extra` groups.
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
    RoleGrants::install(RoleGrantsSnapshot::new(41, groups));
}

fn seeded_plus(extra: Vec<(Group, Vec<Permission>)>) {
    install(|_, _| {}, extra);
}

/// `base`'s seed with `add` added and `remove` taken out; `ApprovalsDecide` follows the kinds.
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
    set.retain(|p| *p != P::ApprovalsDecide);
    if set.iter().any(|p| APPROVAL_KINDS.contains(p)) {
        set.push(P::ApprovalsDecide);
    }
    set
}

/// A row of group `code` (base `base`) at `branch` (`None` = organisation-wide), valid now.
fn row(code: &str, base: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::in_group(
        code,
        base,
        branch.map(BranchId::new),
        Utc::now() - Duration::hours(1),
        None,
    )
}

fn actor(rows: Vec<ScopedRole>) -> Actor {
    Actor::new("u-decider", ORG, rows, None)
}

/// A `CASH_TRANSFER` request from `from_branch`, within the limit or not.
fn transfer(within_limit: bool, from_branch: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: RequestId::new("00000000-0000-0000-0000-0000000000cc"),
        organization_id: ORG.to_owned(),
        type_name: "CASH_TRANSFER".to_owned(),
        branch_id: Some(from_branch.to_owned()),
        object_id: "transfer-1".to_owned(),
        object_version_hash: "h".to_owned(),
        policy_version: "1".to_owned(),
        proposed_payload: json!({ "within_limit": within_limit, "from_branch": from_branch }),
        requested_by: "requester-1".to_owned(),
        requested_at: Utc::now(),
        status: "PENDING".to_owned(),
        idempotency_key: "k".to_owned(),
    }
}

fn refused_with(who: &Actor, request: &ApprovalRequest, code: &str, what: &str) {
    let refusal = TransferApprovalPolicy::new()
        .may_decide(who, request)
        .expect_err(what);
    assert_eq!(refusal.code(), code, "{what}");
}

/// §11.9 A4 (passes today; locks the seed): BRANCH_MANAGER@B1 decides a within-limit transfer from B1 and is refused
/// one above the limit with `ABOVE_LIMIT`.
#[test]
fn ap4_a4_a_branch_manager_decides_within_the_limit_and_not_above() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let bm = actor(vec![row("BRANCH_MANAGER", Role::BranchManager, Some("B1"))]);

    assert_eq!(
        TransferApprovalPolicy::new().may_decide(&bm, &transfer(true, "B1")),
        Ok(())
    );
    refused_with(
        &bm,
        &transfer(false, "B1"),
        "ABOVE_LIMIT",
        "BM@B1 above the limit",
    );
    refused_with(
        &bm,
        &transfer(true, "B2"),
        "WITHIN_LIMIT",
        "BM@B1 within the limit from B2",
    );
}

/// §11.9: the within-limit decision follows the permission. Built-in BRANCH_MANAGER with
/// `ApproveCashTransferWithinLimit` removed is refused `WITHIN_LIMIT`, though its base is BRANCH_MANAGER.
#[test]
fn ap4_a_branch_manager_without_the_within_limit_kind_is_refused_within_limit() {
    let _g = locked();
    let _r = Restore;
    install(
        |r, set| {
            if r == Role::BranchManager {
                set.retain(|p| *p != P::ApproveCashTransferWithinLimit);
            }
        },
        vec![],
    );
    let bm = actor(vec![row("BRANCH_MANAGER", Role::BranchManager, Some("B1"))]);

    refused_with(
        &bm,
        &transfer(true, "B1"),
        "WITHIN_LIMIT",
        "BM@B1 without ApproveCashTransferWithinLimit",
    );
}

/// §11.9 A5: a custom group based on FINANCE, granted `ApproveCashTransferWithinLimit`, with a row at B1, decides a
/// within-limit transfer from B1 (ruling Q2 is now the seed's default, not a fixed rule). Seeded FINANCE is still
/// refused `WITHIN_LIMIT`.
#[test]
fn ap4_a5_a_finance_based_group_granted_within_limit_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("FIN_TRANSFER", Role::Finance),
        set_of(Role::Finance, &[P::ApproveCashTransferWithinLimit], &[]),
    )]);
    let granted = actor(vec![row("FIN_TRANSFER", Role::Finance, Some("B1"))]);
    let seeded_finance = actor(vec![row("FINANCE", Role::Finance, Some("B1"))]);

    assert_eq!(
        TransferApprovalPolicy::new().may_decide(&granted, &transfer(true, "B1")),
        Ok(()),
        "FIN_TRANSFER@B1 decides a within-limit B1 transfer"
    );
    refused_with(
        &granted,
        &transfer(true, "B2"),
        "WITHIN_LIMIT",
        "FIN_TRANSFER@B1 within the limit from B2",
    );
    refused_with(
        &granted,
        &transfer(false, "B1"),
        "ABOVE_LIMIT",
        "FIN_TRANSFER was not granted the above-limit kind",
    );
    refused_with(
        &seeded_finance,
        &transfer(true, "B1"),
        "WITHIN_LIMIT",
        "seeded FINANCE holds no transfer kind (ruling Q2 by seed)",
    );
}

/// §11.9 "`covers` widens the branch kinds on purpose": an organisation-wide row of a group that holds
/// `ApproveCashTransferWithinLimit` decides a within-limit transfer from any branch.
#[test]
fn ap4_an_organisation_wide_row_holding_within_limit_decides_any_source() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("REGION_MGR", Role::BranchManager),
        set_of(Role::BranchManager, &[], &[]),
    )]);
    let region = actor(vec![row("REGION_MGR", Role::BranchManager, None)]);

    for from in ["B1", "B2"] {
        assert_eq!(
            TransferApprovalPolicy::new().may_decide(&region, &transfer(true, from)),
            Ok(()),
            "an org-wide REGION_MGR row covers {from}"
        );
    }
}

/// §11.9 A6 (policy level; passes today): EXECUTIVE@B1 (branch-scoped) decides an above-limit transfer from B1.
#[test]
fn ap4_a6_a_branch_scoped_executive_decides_above_the_limit_from_its_branch() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let exec = actor(vec![row("EXECUTIVE", Role::Executive, Some("B1"))]);

    assert_eq!(
        TransferApprovalPolicy::new().may_decide(&exec, &transfer(false, "B1")),
        Ok(())
    );
}

/// §11.9 "the above-limit transfer is a branch kind": a branch-scoped custom group granted
/// `ApproveCashTransferAboveLimit` at B1 decides an above-limit transfer from B1, and is refused one from B2 with
/// `ABOVE_LIMIT`.
#[test]
fn ap4_a_branch_scoped_group_granted_above_limit_decides_only_from_its_branch() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("BIG_TRANSFER", Role::BranchManager),
        set_of(
            Role::BranchManager,
            &[P::ApproveCashTransferAboveLimit],
            &[],
        ),
    )]);
    let big = actor(vec![row("BIG_TRANSFER", Role::BranchManager, Some("B1"))]);

    assert_eq!(
        TransferApprovalPolicy::new().may_decide(&big, &transfer(false, "B1")),
        Ok(()),
        "BIG_TRANSFER@B1 decides above the limit from B1"
    );
    refused_with(
        &big,
        &transfer(false, "B2"),
        "ABOVE_LIMIT",
        "BIG_TRANSFER@B1 above the limit from B2",
    );
}

/// §11.9: the above-limit decision follows the permission, not the EXECUTIVE base. EXECUTIVE with
/// `ApproveCashTransferAboveLimit` removed (a test snapshot; rule 4 keeps the real group fixed) is refused
/// `ABOVE_LIMIT`.
#[test]
fn ap4_an_executive_without_the_above_limit_kind_is_refused_above_limit() {
    let _g = locked();
    let _r = Restore;
    install(
        |r, set| {
            if r == Role::Executive {
                set.retain(|p| *p != P::ApproveCashTransferAboveLimit);
            }
        },
        vec![],
    );
    let exec = actor(vec![row("EXECUTIVE", Role::Executive, None)]);

    refused_with(
        &exec,
        &transfer(false, "B1"),
        "ABOVE_LIMIT",
        "EXECUTIVE without ApproveCashTransferAboveLimit",
    );
}
