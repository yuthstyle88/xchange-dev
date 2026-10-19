//! Ticket #874 — the approval and reversal route rows: a branch manager and FINANCE may reach
//! `approvalDecide` and `approvalRead` (each type's own policy still decides), and a teller or a
//! branch manager may reach `reversalRequest`.
//! Written before the implementation by the ticket author (claude/f03345fe). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Why:
//! - the owner's answer T2 (`docs/design/blueprint/quote-transaction.md` §0, §5.1): the branch's
//!   BRANCH_MANAGER approves an exchange above the limit;
//! - the owner's answer R-3 (`docs/design/blueprint/reversal.md` §0, §5): FINANCE approves a
//!   reversal;
//! - `reversal.md` §4.2: TELLER and BRANCH_MANAGER request a reversal (Blueprint `ReversalRequest`,
//!   already theirs in `role_permissions.rs`). No route row admitted any of them.
//!
//! Lane controller's rulings (claude/608dff, 2026-10-10): G4 of #672; RG3 and RG4 of #682, folded
//! into this ticket. Rates stay the owner's through `RatePublicationPolicy`
//! (`crates/xc-rate/tests/rate_publication_manager_test.rs`), and the queue is read per type
//! (`api/tests/approvals_read_per_type_test.rs`).
//!
//! Two tables answer today, and both change:
//! - the v3 table, `Policy::allow` (the gate every v3 route goes through, `view.rs:349`). Its staff
//!   arm admits, on Rpc:
//!   - `approvalDecide`/`approvalRead` for `MANAGER`, `BACKEND` (they become BRANCH_MANAGER,
//!     `access/v3_roles.rs:14`) and `FINANCE`;
//!   - `reversalRequest` for `FRONT` (TELLER), `MANAGER` and `BACKEND`;
//! - the Blueprint table, `Policy::authorize` (what OR-4b #767 points the routes at):
//!   - `RoleGrants::seed(BranchManager)` and `of(Finance)` hold `ApprovalsDecide`;
//!   - `Policy::need(Rpc, "reversalRequest")` is `Permission::ReversalRequest`.
//!
//! Signatures: none new. The action name `reversalRequest` is [required]: RV-2's command inherits
//! `(Rpc, "reversalRequest")` (#682).

mod policy_common;

use policy_common::*;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::policy::value_objects::actor::Actor as V3Actor;
use xc_auth::domain::policy::value_objects::role::Role as V3Role;

const B2: &str = "B2";
const APPROVAL: [&str; 2] = ["approvalDecide", "approvalRead"];
const REVERSAL: &str = "reversalRequest";

const V3_STAFF: [V3Role; 9] = [
    V3Role::Front,
    V3Role::Backend,
    V3Role::Manager,
    V3Role::Rates,
    V3Role::Monitor,
    V3Role::Finance,
    V3Role::System,
    V3Role::Documents,
    V3Role::DocumentReview,
];

fn v3_staff(role: V3Role) -> V3Actor {
    V3Actor::staff(format!("emp-{}", role.as_str()), role, BranchId::new(B2)).expect("a staff role")
}

fn v3_ask(who: &V3Actor, on: EntryPath, name: &str) -> Result<(), PolicyError> {
    Policy::allow(Some(who), on, &action(name))
}

fn blueprint_allowed(on: EntryPath, name: &str) -> Vec<&'static str> {
    BLUEPRINT
        .into_iter()
        .filter(|r| ask(Some(&staff(*r, B2)), on, name).is_ok())
        .map(Role::as_str)
        .collect()
}

fn v3_allowed(on: EntryPath, name: &str) -> Vec<&'static str> {
    V3_STAFF
        .into_iter()
        .filter(|r| v3_ask(&v3_staff(*r), on, name).is_ok())
        .map(V3Role::as_str)
        .collect()
}

#[test]
fn approvals_decide_is_held_by_the_executive_branch_manager_and_finance() {
    // §11.2 "Branch cash basics": EXECUTIVE, BRANCH_MANAGER and FINANCE hold ApprovalsDecide
    let holders: Vec<&str> = BLUEPRINT
        .into_iter()
        .filter(|r| RoleGrants::seed(*r).contains(&Permission::ApprovalsDecide))
        .map(Role::as_str)
        .collect();
    assert_eq!(holders, ["EXECUTIVE", "BRANCH_MANAGER", "FINANCE"]);
}

#[test]
fn under_the_blueprint_table_approvals_are_reached_by_those_three_on_rpc_only() {
    // §11.2 "Branch cash basics": compute ApprovalsDecide holders from RoleGrants::seed
    let holders: Vec<&str> = BLUEPRINT
        .into_iter()
        .filter(|r| RoleGrants::seed(*r).contains(&Permission::ApprovalsDecide))
        .map(Role::as_str)
        .collect();
    for name in APPROVAL {
        assert_eq!(
            blueprint_allowed(EntryPath::Rpc, name),
            holders,
            "{name} on Rpc"
        );
        assert!(
            blueprint_allowed(EntryPath::StaffRpc, name).is_empty(),
            "{name}: still nobody on StaffRpc"
        );
        for r in BLUEPRINT
            .into_iter()
            .filter(|r| !RoleGrants::seed(*r).contains(&Permission::ApprovalsDecide))
        {
            assert_eq!(
                ask(Some(&staff(r, B2)), EntryPath::Rpc, name),
                Err(PolicyError::RoleNotAllowedHere),
                "{name}: {} on Rpc is refused as before",
                r.as_str()
            );
        }
    }
}

#[test]
fn under_the_v3_table_manager_backend_and_finance_reach_approvals_on_rpc() {
    for name in APPROVAL {
        assert_eq!(
            Policy::allow(
                Some(&V3Actor::owner("owner-1")),
                EntryPath::Rpc,
                &action(name)
            ),
            Ok(()),
            "{name}: the owner, as before"
        );
        assert_eq!(
            v3_allowed(EntryPath::Rpc, name),
            ["BACKEND", "MANAGER", "FINANCE"],
            "{name} on Rpc"
        );
        for r in V3_STAFF
            .into_iter()
            .filter(|r| ![V3Role::Manager, V3Role::Backend, V3Role::Finance].contains(r))
        {
            assert_eq!(
                v3_ask(&v3_staff(r), EntryPath::Rpc, name),
                Err(PolicyError::RoleNotAllowedHere),
                "{name}: {} on Rpc is refused as before",
                r.as_str()
            );
        }
    }
}

#[test]
fn a_reversal_request_needs_the_reversal_request_permission_on_rpc() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &action(REVERSAL)),
        Some(Need::Permission(Permission::ReversalRequest)),
        "the Blueprint row"
    );
    // §11.2: ReversalRequest is CUSTOMERS CREATE, held by EXECUTIVE, RATE_MANAGER, BRANCH_MANAGER, TELLER
    assert_eq!(
        blueprint_allowed(EntryPath::Rpc, REVERSAL),
        ["EXECUTIVE", "RATE_MANAGER", "BRANCH_MANAGER", "TELLER"],
        "the roles that hold ReversalRequest (§11.2)"
    );
    assert_eq!(
        v3_allowed(EntryPath::Rpc, REVERSAL),
        ["FRONT", "BACKEND", "MANAGER"],
        "the v3 roles that become TELLER or BRANCH_MANAGER"
    );
    for r in V3_STAFF
        .into_iter()
        .filter(|r| ![V3Role::Front, V3Role::Manager, V3Role::Backend].contains(r))
    {
        assert_eq!(
            v3_ask(&v3_staff(r), EntryPath::Rpc, REVERSAL),
            Err(PolicyError::RoleNotAllowedHere),
            "{} may not request a reversal",
            r.as_str()
        );
    }
    assert_eq!(
        Policy::allow(
            Some(&V3Actor::owner("owner-1")),
            EntryPath::Rpc,
            &action(REVERSAL)
        ),
        Err(PolicyError::InvalidRequest),
        "the owner's session holds no ReversalRequest (EXECUTIVE, ADMIN), as on every v3 row it lacks"
    );
}
