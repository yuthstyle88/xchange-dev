//! A FINANCE user of the whole organisation decides a COUNT_DIFFERENCE request of any branch. Design
//! `docs/design/blueprint/ledger-inventory.md:364`: "`may_decide` ⇔ FINANCE (organisation-wide or of the
//! branch); `allows_self_decision` = false".
//!
//! Written before the fix by the ticket author (claude/c4d72c). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! `CountDifferencePolicy::may_decide` is called directly: `ApprovalService::decide` already routes every decision
//! through it (`ledger_approvals_test.rs`, `count_verify_gate_test.rs`). A role's scope follows the `Authorizer`'s
//! rule (`xc-auth/src/domain/access/authorizer.rs`): a role row with no branch covers every branch.

use chrono::{TimeZone, Utc};
use serde_json::json;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::ledger_approval::count_difference::CountDifferencePolicy;
use xc_kernel::ids::BranchId;

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
        type_name: "COUNT_DIFFERENCE".to_owned(),
        branch_id: branch.map(str::to_owned),
        object_id: "count-1".to_owned(),
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
fn an_organisation_wide_finance_decides_a_count_difference_of_any_branch() {
    let who = actor(&[(Role::Finance, None)]);
    for branch in ["MAIN", "B2"] {
        assert!(
            CountDifferencePolicy
                .may_decide(&who, &request(Some(branch)))
                .is_ok(),
            "FINANCE of the whole organisation decides {branch}'s count difference (ledger-inventory.md:364)"
        );
    }
}

#[test]
fn a_branch_finance_decides_only_its_own_branch() {
    let who = actor(&[(Role::Finance, Some("MAIN"))]);
    assert!(
        CountDifferencePolicy
            .may_decide(&who, &request(Some("MAIN")))
            .is_ok()
    );
    let other = CountDifferencePolicy
        .may_decide(&who, &request(Some("B2")))
        .expect_err("FINANCE of MAIN does not decide B2's count difference");
    assert_eq!(other.code(), "outOfScope");
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
        let refused = CountDifferencePolicy
            .may_decide(&who, &request(Some("MAIN")))
            .expect_err("only FINANCE decides a count difference");
        assert_eq!(refused.code(), "outOfScope", "{role:?}");
    }
}
