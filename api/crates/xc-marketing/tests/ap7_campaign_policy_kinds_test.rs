//! Up-front tests for AP-7 (design `docs/design/blueprint/user-management-rbac.md` §11.9): `CAMPAIGN_VERSION`'s
//! `CampaignApprovalPolicy::may_decide` decides by `ApproveCampaignVersion` through `ApprovalDecider`, not by the
//! row's base role.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9 for this kind: an organisation kind, `holds_anywhere(who, ApproveCampaignVersion)`. The proposer check
//! (D-5, `proposerCannotDecide`) stays first, then `outOfScope`; `allows_self_decision` stays false.
//!
//! The policy holds a pool it never touches, so a lazy one that never connects will do (as
//! `xc-rate/tests/rate_publication_manager_test.rs`). Tests marked "passes today" lock the seed's outcome (A1, A9);
//! the rest fail until AP-7 and AP-1 land.
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
use xc_marketing::application::approval::CampaignApprovalPolicy;

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

const PROPOSER: &str = "u-proposer";

fn policy() -> CampaignApprovalPolicy {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://nobody@127.0.0.1:1/never_connected_test")
        .expect("a lazy pool");
    CampaignApprovalPolicy::new(pool)
}

fn campaign() -> ApprovalRequest {
    request("CAMPAIGN_VERSION", None, PROPOSER)
}

fn refused_with(who: &Actor, code: &str, what: &str) {
    let refusal = policy().may_decide(who, &campaign()).expect_err(what);
    assert_eq!(refusal.code(), code, "{what}");
}

/// §11.9 A9 (passes today): a proposer who holds `ApproveCampaignVersion` (seeded EXECUTIVE, or a custom group
/// granted it) is still refused `proposerCannotDecide`, and may not close the request at submit.
#[tokio::test]
async fn ap7_a9_a_proposer_holding_the_kind_is_still_refused() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("CAMPAIGN_LEAD", Role::Marketing),
        set_of(Role::Marketing, &[P::ApproveCampaignVersion], &[]),
    )]);
    for proposer in [
        actor_as(PROPOSER, vec![row("EXECUTIVE", Role::Executive, None)]),
        actor_as(PROPOSER, vec![row("CAMPAIGN_LEAD", Role::Marketing, None)]),
    ] {
        refused_with(&proposer, "proposerCannotDecide", "the proposer");
        assert!(
            !policy().allows_self_decision(&proposer),
            "no self decision"
        );
    }
}

/// §11.9: a custom group (base MARKETING) granted `ApproveCampaignVersion` decides a campaign version.
#[tokio::test]
async fn ap7_a_custom_group_granted_the_kind_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![(
        custom("CAMPAIGN_LEAD", Role::Marketing),
        set_of(Role::Marketing, &[P::ApproveCampaignVersion], &[]),
    )]);
    let lead = actor(vec![row("CAMPAIGN_LEAD", Role::Marketing, Some("B1"))]);

    assert_eq!(policy().may_decide(&lead, &campaign()), Ok(()));
}

/// §11.9 A6 (passes today): EXECUTIVE@B1 (branch-scoped) decides a campaign version (organisation scope).
#[tokio::test]
async fn ap7_a6_a_branch_scoped_executive_decides() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let exec = actor(vec![row("EXECUTIVE", Role::Executive, Some("B1"))]);

    assert_eq!(policy().may_decide(&exec, &campaign()), Ok(()));
}

/// §11.9 (non-holder; passes today for the seed): seeded MARKETING holds no kind and is refused `outOfScope`.
#[tokio::test]
async fn ap7_a_non_holder_is_refused_out_of_scope() {
    let _g = locked();
    let _r = Restore;
    seeded_plus(vec![]);
    let marketing = actor(vec![row("MARKETING", Role::Marketing, None)]);

    refused_with(&marketing, "outOfScope", "seeded MARKETING");
}

/// §11.9: the decision follows the permission, not the EXECUTIVE base. EXECUTIVE with `ApproveCampaignVersion`
/// removed (a test snapshot; rule 4 keeps the real group fixed) is refused `outOfScope`.
#[tokio::test]
async fn ap7_an_executive_without_the_kind_is_refused_out_of_scope() {
    let _g = locked();
    let _r = Restore;
    seeded_without(Role::Executive, P::ApproveCampaignVersion);
    let exec = actor(vec![row("EXECUTIVE", Role::Executive, None)]);

    refused_with(
        &exec,
        "outOfScope",
        "EXECUTIVE without ApproveCampaignVersion",
    );
}
