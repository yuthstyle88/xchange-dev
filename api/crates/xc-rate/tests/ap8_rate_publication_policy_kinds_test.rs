//! Up-front tests for AP-8 (design `docs/design/blueprint/user-management-rbac.md` §11.9): `RATE_PUBLICATION`'s
//! `RatePublicationPolicy` decides, and lets a proposer close at submit, by `ApproveRatePublication` through
//! `ApprovalDecider`, not by the row's base role.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9 for this kind: an organisation kind. `may_decide` is `holds_anywhere(who, ApproveRatePublication)`, else
//! `outOfScope`; `allows_self_decision` (`rate_publication.rs:102-104`, a role test too) is the same
//! `holds_anywhere`: "a proposer holding it may close at submit".
//!
//! The policy holds a pool it never touches, so a lazy one that never connects will do
//! (`rate_publication_manager_test.rs` pins the seed's outcomes; they stay green). Tests marked "passes today" lock
//! the seed's outcome (A1, A6); the rest fail until AP-8 and AP-1 land.
//!
//! No database. `RoleGrants::install` is process-global: every test here holds `LOCK` and puts the seeded snapshot
//! back when it passes. CI runs nextest, one process per test, so other files' snapshots never meet these; under a
//! plain `cargo test` the package's files share one binary and may interleave.

use std::sync::Mutex;

use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::access::permission_module::APPROVAL_KINDS;
use xc_auth::domain::access::{
    Actor, Group, Permission, Role, RoleGrants, RoleGrantsSnapshot, ScopedRole,
};
use xc_kernel::ids::BranchId;
use xc_rate::application::approval::rate_publication::RatePublicationPolicy;

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

fn policy() -> RatePublicationPolicy {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://nobody@127.0.0.1:1/never_connected_test")
        .expect("a lazy pool");
    RatePublicationPolicy::new(pool)
}

fn publication() -> ApprovalRequest {
    request("RATE_PUBLICATION", Some("MAIN"), "rates-1")
}

fn refused_out_of_scope(who: &Actor, what: &str) {
    let refusal = policy().may_decide(who, &publication()).expect_err(what);
    assert_eq!(refusal.code(), "outOfScope", "{what}");
}

/// §11.9: a custom group (base RATE_MANAGER) granted `ApproveRatePublication` decides a rate publication and, as a
/// proposer, may close it at submit.
#[tokio::test]
async fn ap8_a_custom_group_granted_the_kind_decides_and_self_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("RATE_LEAD", Role::RateManager),
        set_of(Role::RateManager, &[P::ApproveRatePublication], &[]),
    )]);
    let lead = actor(vec![row("RATE_LEAD", Role::RateManager, Some("MAIN"))]);

    assert_eq!(policy().may_decide(&lead, &publication()), Ok(()));
    assert!(
        policy().allows_self_decision(&lead),
        "a proposer holding ApproveRatePublication may close at submit"
    );
}

/// §11.9 (non-holder; passes today for the seed): seeded RATE_MANAGER holds no kind: refused `outOfScope`, and may
/// not close its own request.
#[tokio::test]
async fn ap8_a_non_holder_is_refused_and_does_not_self_decide() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let rates = actor(vec![row("RATE_MANAGER", Role::RateManager, None)]);

    refused_out_of_scope(&rates, "seeded RATE_MANAGER");
    assert!(!policy().allows_self_decision(&rates), "no self decision");
}

/// §11.9: both answers follow the permission, not the EXECUTIVE base. EXECUTIVE with `ApproveRatePublication`
/// removed (a test snapshot; rule 4 keeps the real group fixed) is refused `outOfScope` and may not self-decide.
#[tokio::test]
async fn ap8_an_executive_without_the_kind_neither_decides_nor_self_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_without(Role::Executive, P::ApproveRatePublication);
    let exec = actor(vec![row("EXECUTIVE", Role::Executive, None)]);

    refused_out_of_scope(&exec, "EXECUTIVE without ApproveRatePublication");
    assert!(
        !policy().allows_self_decision(&exec),
        "EXECUTIVE without ApproveRatePublication may not close at submit"
    );
}

/// §11.9 A6 (passes today): EXECUTIVE@B1 (branch-scoped) decides a rate publication (organisation scope) and may
/// close its own.
#[tokio::test]
async fn ap8_a6_a_branch_scoped_executive_decides_and_self_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let exec = actor(vec![row("EXECUTIVE", Role::Executive, Some("B1"))]);

    assert_eq!(policy().may_decide(&exec, &publication()), Ok(()));
    assert!(policy().allows_self_decision(&exec));
}
