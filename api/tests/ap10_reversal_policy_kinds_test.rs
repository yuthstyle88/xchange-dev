//! Up-front tests for AP-10 (design `docs/design/blueprint/user-management-rbac.md` §11.9): `TRANSACTION_REVERSAL`'s
//! `ReversalPolicy::may_decide` decides by `ApproveTransactionReversal` through `ApprovalDecider`, not by the row's
//! base role.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9 for this kind: a branch kind, `holds_on(who, ApproveTransactionReversal, request.branch_id)`, else the one
//! `outOfScope` it answers today; `allows_self_decision` stays false.
//! `org_wide_finance_decides_reversal_test.rs` pins the seed's outcomes and stays green. Tests marked "passes today"
//! lock the seed's outcome (A1); the rest fail until AP-10 and AP-1 land.
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
use xc_auth::domain::access::{Actor, Group, Permission, Role, RoleGrants, RoleGrantsSnapshot, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::application::workflows::exchange::reversal_approval::ReversalPolicy;

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

/// The seed with `kind` taken off the built-in group `role`.
fn seeded_without(role: Role, kind: Permission) {
    install(
        move |r, set| {
            if r == role {
                set.retain(|p| *p != kind);
            }
        },
        vec![],
    );
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
    actor_as("u-decider", rows)
}

fn actor_as(user_id: &str, rows: Vec<ScopedRole>) -> Actor {
    Actor::new(user_id, ORG, rows, None)
}

/// A request of `type_name` at `branch`, proposed by `requested_by`.
fn request(type_name: &str, branch: Option<&str>, requested_by: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: RequestId::new("00000000-0000-0000-0000-0000000000dd"),
        organization_id: ORG.to_owned(),
        type_name: type_name.to_owned(),
        branch_id: branch.map(str::to_owned),
        object_id: "object-1".to_owned(),
        object_version_hash: "h".to_owned(),
        policy_version: "1".to_owned(),
        proposed_payload: json!({}),
        requested_by: requested_by.to_owned(),
        requested_at: Utc::now(),
        status: "PENDING".to_owned(),
        idempotency_key: "k".to_owned(),
    }
}

fn reversal(branch: &str) -> ApprovalRequest {
    request("TRANSACTION_REVERSAL", Some(branch), "teller-1")
}

fn refused(who: &Actor, branch: &str, what: &str) {
    let refusal = ReversalPolicy
        .may_decide(who, &reversal(branch))
        .expect_err(what);
    assert_eq!(refusal.code(), "outOfScope", "{what}");
    assert_eq!(
        refusal.message(),
        "FINANCE of the branch decides a reversal",
        "{what}"
    );
}

/// §11.9 A1 (passes today; locks the seed): FINANCE@B1 decides a B1 reversal and is refused a B2 one.
#[test]
fn ap10_a1_finance_decides_its_own_branch() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let fin = actor(vec![row("FINANCE", Role::Finance, Some("B1"))]);

    assert_eq!(ReversalPolicy.may_decide(&fin, &reversal("B1")), Ok(()));
    refused(&fin, "B2", "FINANCE@B1 on B2");
}

/// §11.9 A2: custom group `NIGHT_FIN`, base TELLER, granted `ApproveTransactionReversal`, row at B1. It decides a B1
/// reversal and is refused a B2 one with `outOfScope`.
#[test]
fn ap10_a2_night_fin_decides_its_own_branch_only() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("NIGHT_FIN", Role::Teller),
        set_of(Role::Teller, &[P::ApproveTransactionReversal], &[]),
    )]);
    let night = actor(vec![row("NIGHT_FIN", Role::Teller, Some("B1"))]);

    assert_eq!(
        ReversalPolicy.may_decide(&night, &reversal("B1")),
        Ok(()),
        "NIGHT_FIN@B1 decides a B1 reversal"
    );
    refused(&night, "B2", "NIGHT_FIN@B1 on B2");
    assert!(!ReversalPolicy.allows_self_decision(&night), "no self decision");
}

/// §11.9: the decision follows the permission. Built-in FINANCE with `ApproveTransactionReversal` removed is refused
/// a B1 reversal, though its base is FINANCE.
#[test]
fn ap10_finance_with_the_kind_removed_is_refused() {
    let _g = locked();
    let _r = Restore;
    seeded_without(Role::Finance, P::ApproveTransactionReversal);
    let fin = actor(vec![row("FINANCE", Role::Finance, Some("B1"))]);

    refused(&fin, "B1", "FINANCE@B1 without ApproveTransactionReversal");
}
