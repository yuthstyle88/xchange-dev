//! A FINANCE user of the whole organisation decides a reversal of any branch. Design
//! `docs/design/blueprint/reversal.md:178-179`: "`may_decide(who, request)` ⇔ `who` holds **FINANCE** for the
//! reversal's branch (organisation-wide or that branch, `Authorizer` scope rules)".
//!
//! Written before the fix by the ticket author (claude/c4d72c). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! `ReversalPolicy::may_decide` is called directly: `ApprovalService::decide` already routes every decision
//! through it (`reversal_request_test.rs` drives that path). A role's scope follows the `Authorizer`'s rule
//! (`xc-auth/src/domain/access/authorizer.rs`): a role row with no branch covers every branch.

use chrono::{TimeZone, Utc};
use serde_json::json;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::application::workflows::exchange::reversal_approval::ReversalPolicy;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn actor(roles: &[(Role, Option<&str>)]) -> Actor {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        "decider",
        ORG,
        roles
            .iter()
            .map(|(r, b)| ScopedRole::new(*r, b.map(BranchId::new), from, None))
            .collect(),
        None,
    )
}

fn request(branch: Option<&str>) -> ApprovalRequest {
    ApprovalRequest {
        id: RequestId::new("req-1"),
        organization_id: ORG.to_owned(),
        type_name: "TRANSACTION_REVERSAL".to_owned(),
        branch_id: branch.map(str::to_owned),
        object_id: "rev-1".to_owned(),
        object_version_hash: "hash".to_owned(),
        policy_version: "v1".to_owned(),
        proposed_payload: json!({}),
        requested_by: "requester".to_owned(),
        requested_at: Utc.with_ymd_and_hms(2026, 10, 15, 2, 0, 0).unwrap(),
        status: "PENDING".to_owned(),
        idempotency_key: "key".to_owned(),
    }
}

#[test]
fn an_organisation_wide_finance_decides_a_reversal_of_any_branch() {
    let who = actor(&[(Role::Finance, None)]);
    for branch in ["MAIN", "B2"] {
        assert!(
            ReversalPolicy
                .may_decide(&who, &request(Some(branch)))
                .is_ok(),
            "FINANCE of the whole organisation decides {branch}'s reversal (reversal.md:178-179)"
        );
    }
}

#[test]
fn an_organisation_wide_finance_decides_a_reversal_with_no_branch() {
    let who = actor(&[(Role::Finance, None)]);
    assert!(ReversalPolicy.may_decide(&who, &request(None)).is_ok());
}

#[test]
fn a_branch_finance_decides_only_its_own_branch() {
    let who = actor(&[(Role::Finance, Some("MAIN"))]);
    assert!(
        ReversalPolicy
            .may_decide(&who, &request(Some("MAIN")))
            .is_ok()
    );
    let other = ReversalPolicy
        .may_decide(&who, &request(Some("B2")))
        .expect_err("FINANCE of MAIN does not decide B2's reversal");
    assert_eq!(other.code(), "outOfScope");
    let none = ReversalPolicy
        .may_decide(&who, &request(None))
        .expect_err("FINANCE of one branch does not decide a reversal that names no branch");
    assert_eq!(none.code(), "outOfScope");
}

#[test]
fn only_finance_decides() {
    for role in [
        Role::Executive,
        Role::Admin,
        Role::BranchManager,
        Role::Teller,
    ] {
        let who = actor(&[(role, None)]);
        let refused = ReversalPolicy
            .may_decide(&who, &request(Some("MAIN")))
            .expect_err("only FINANCE decides a reversal");
        assert_eq!(refused.code(), "outOfScope", "{role:?}");
    }
}
