//! Up-front tests for AP-6 (design `docs/design/blueprint/user-management-rbac.md` §11.9): `NEGATIVE_STOCK`'s
//! `NegativeStockPolicy::may_decide` decides by `ApproveNegativeStock` through `ApprovalDecider`, not by the row's
//! base role.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9 for this kind: a branch kind, `holds_on(who, ApproveNegativeStock, request.branch_id)`. The two
//! `outOfScope` messages stay exactly as today:
//! - "only branch manager may decide negative stock" when the caller holds the kind nowhere (`!holds_anywhere`);
//! - "branch manager can only decide requests for their own branch" when it holds it, but not on the request's
//!   branch (`holds_anywhere && !holds_on`).
//!
//! `covers` widens this kind on purpose (§11.9): an organisation-wide row of a group that holds it decides any
//! branch. Tests marked "passes today" lock the seed's outcome (A1); the rest fail until AP-6 and AP-1 land.
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
use xc_cash::application::ledger_approval::negative_stock::NegativeStockPolicy;
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

const ONLY: &str = "only branch manager may decide negative stock";
const OWN_BRANCH: &str = "branch manager can only decide requests for their own branch";

fn negative_stock(branch: &str) -> ApprovalRequest {
    request("NEGATIVE_STOCK", Some(branch), "requester-1")
}

fn refused_with(who: &Actor, branch: &str, message: &str, what: &str) {
    let refusal = NegativeStockPolicy
        .may_decide(who, &negative_stock(branch))
        .expect_err(what);
    assert_eq!(refusal.code(), "outOfScope", "{what}");
    assert_eq!(refusal.message(), message, "{what}");
}

/// §11.9 A1 (passes today; locks the seed): BRANCH_MANAGER@B1 decides a B1 request and is refused a B2 one with the
/// "own branch" message; FINANCE is refused with the "only" message.
#[test]
fn ap6_a1_the_seed_decides_as_today() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let bm = actor(vec![row("BRANCH_MANAGER", Role::BranchManager, Some("B1"))]);

    assert_eq!(
        NegativeStockPolicy.may_decide(&bm, &negative_stock("B1")),
        Ok(())
    );
    refused_with(&bm, "B2", OWN_BRANCH, "BRANCH_MANAGER@B1 on B2");
    let fin = actor(vec![row("FINANCE", Role::Finance, Some("B1"))]);
    refused_with(&fin, "B1", ONLY, "FINANCE holds no negative-stock kind");
}

/// §11.9 (A3's shape): built-in BRANCH_MANAGER with `ApproveNegativeStock` removed. A BRANCH_MANAGER@B1 user is
/// refused a B1 request with the "only branch manager" message, though its base is BRANCH_MANAGER.
#[test]
fn ap6_a_branch_manager_with_the_kind_removed_is_refused() {
    let _g = locked();
    let _r = Restore;
    seeded_without(Role::BranchManager, P::ApproveNegativeStock);
    let bm = actor(vec![row("BRANCH_MANAGER", Role::BranchManager, Some("B1"))]);

    refused_with(
        &bm,
        "B1",
        ONLY,
        "BRANCH_MANAGER@B1 without ApproveNegativeStock",
    );
}

/// §11.9: a custom group (base TELLER) granted `ApproveNegativeStock`, with a row at B1, decides a B1 request and is
/// refused a B2 one with the "own branch" message.
#[test]
fn ap6_a_custom_group_granted_the_kind_decides_its_own_branch_only() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("SHIFT_LEAD", Role::Teller),
        set_of(Role::Teller, &[P::ApproveNegativeStock], &[]),
    )]);
    let lead = actor(vec![row("SHIFT_LEAD", Role::Teller, Some("B1"))]);

    assert_eq!(
        NegativeStockPolicy.may_decide(&lead, &negative_stock("B1")),
        Ok(()),
        "SHIFT_LEAD@B1 decides B1"
    );
    refused_with(&lead, "B2", OWN_BRANCH, "SHIFT_LEAD@B1 on B2");
}

/// §11.9 "`covers` widens the branch kinds on purpose": an organisation-wide row of a group holding
/// `ApproveNegativeStock` decides a request of any branch.
#[test]
fn ap6_an_organisation_wide_row_holding_the_kind_decides_any_branch() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("STOCK_DESK", Role::Finance),
        set_of(Role::Finance, &[P::ApproveNegativeStock], &[]),
    )]);
    let desk = actor(vec![row("STOCK_DESK", Role::Finance, None)]);

    for branch in ["B1", "B2"] {
        assert_eq!(
            NegativeStockPolicy.may_decide(&desk, &negative_stock(branch)),
            Ok(()),
            "an org-wide STOCK_DESK row covers {branch}"
        );
    }
}
