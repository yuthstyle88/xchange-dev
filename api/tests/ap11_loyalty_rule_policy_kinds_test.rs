//! Up-front tests for AP-11 (design `docs/design/blueprint/user-management-rbac.md` §11.9): `LOYALTY_RULE_VERSION`'s
//! `LoyaltyRuleVersionPolicy::may_decide` decides by `ApproveLoyaltyRuleVersion` through `ApprovalDecider`, not by
//! the row's base role.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9 for this kind: an organisation kind, `holds_anywhere(who, ApproveLoyaltyRuleVersion)`, else the one
//! `outOfScope` it answers today; `allows_self_decision` stays false. Tests marked "passes today" lock the seed's
//! outcome (A1, A6); the rest fail until AP-11 and AP-1 land.
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
use xchange::application::workflows::loyalty::rule_approval::LoyaltyRuleVersionPolicy;

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

fn rule_version() -> ApprovalRequest {
    request("LOYALTY_RULE_VERSION", None, "marketing-1")
}

fn refused(who: &Actor, what: &str) {
    let refusal = LoyaltyRuleVersionPolicy::new()
        .may_decide(who, &rule_version())
        .expect_err(what);
    assert_eq!(refusal.code(), "outOfScope", "{what}");
    assert_eq!(
        refusal.message(),
        "only EXECUTIVE may decide loyalty rule versions",
        "{what}"
    );
}

/// §11.9: a custom group (base MARKETING) granted `ApproveLoyaltyRuleVersion` decides a loyalty rule version, and
/// still may not close its own.
#[test]
fn ap11_a_custom_group_granted_the_kind_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("LOYALTY_LEAD", Role::Marketing),
        set_of(Role::Marketing, &[P::ApproveLoyaltyRuleVersion], &[]),
    )]);
    let lead = actor(vec![row("LOYALTY_LEAD", Role::Marketing, None)]);

    assert_eq!(
        LoyaltyRuleVersionPolicy::new().may_decide(&lead, &rule_version()),
        Ok(())
    );
    assert!(
        !LoyaltyRuleVersionPolicy::new().allows_self_decision(&lead),
        "no self decision"
    );
}

/// §11.9 (non-holder; passes today for the seed): seeded MARKETING and BRANCH_MANAGER hold no kind and are refused.
#[test]
fn ap11_a_non_holder_is_refused() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    refused(
        &actor(vec![row("MARKETING", Role::Marketing, None)]),
        "seeded MARKETING",
    );
    refused(
        &actor(vec![row("BRANCH_MANAGER", Role::BranchManager, Some("B1"))]),
        "seeded BRANCH_MANAGER",
    );
}

/// §11.9: the decision follows the permission, not the EXECUTIVE base. EXECUTIVE with `ApproveLoyaltyRuleVersion`
/// removed (a test snapshot; rule 4 keeps the real group fixed) is refused.
#[test]
fn ap11_an_executive_without_the_kind_is_refused() {
    let _g = locked();
    let _r = Restore;
    seeded_without(Role::Executive, P::ApproveLoyaltyRuleVersion);
    refused(
        &actor(vec![row("EXECUTIVE", Role::Executive, None)]),
        "EXECUTIVE without ApproveLoyaltyRuleVersion",
    );
}

/// §11.9 A6 (passes today): EXECUTIVE@B1 (branch-scoped) decides a loyalty rule version (organisation scope).
#[test]
fn ap11_a6_a_branch_scoped_executive_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let exec = actor(vec![row("EXECUTIVE", Role::Executive, Some("B1"))]);

    assert_eq!(
        LoyaltyRuleVersionPolicy::new().may_decide(&exec, &rule_version()),
        Ok(())
    );
}
