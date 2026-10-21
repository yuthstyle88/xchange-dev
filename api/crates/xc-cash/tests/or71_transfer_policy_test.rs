//! Ticket #1020 (OR-7.1) — `TransferApprovalPolicy` on `access::Actor`, asked directly (design
//! `docs/design/blueprint/org-roles-branches.md` §3.3; A5 with ruling Q2 of #582). Written before the
//! implementation by the ticket author (claude/89bd61d9, rule A). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! `transfer_flow_test.rs` and `moves_test.rs` pin these outcomes through `TransferFlow::decide`, which first
//! narrows the caller to the transfer's source (`policy_actor`). This file asks the policy itself, so the rule
//! holds whatever actor a caller hands it ([required]):
//! - within the limit: a BRANCH_MANAGER of the **source** branch decides; a BRANCH_MANAGER of another branch, the
//!   EXECUTIVE (v3 owner: EXECUTIVE + ADMIN) and FINANCE (though it holds `ApprovalsDecide`) are refused
//!   `WITHIN_LIMIT`;
//! - above the limit, or with no `within_limit` in the payload: only the EXECUTIVE decides; a BRANCH_MANAGER of
//!   the source is refused `ABOVE_LIMIT`;
//! - no self-decision;
//! - EXECUTIVE alone (no ADMIN) is enough above the limit.
//!
//! No database.

use chrono::{DateTime, Utc};
use serde_json::json;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::approval::TransferApprovalPolicy;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn user(id: &str, roles: &[(Role, Option<&str>)]) -> Actor {
    Actor::new(
        id,
        ORG,
        roles
            .iter()
            .map(|(role, branch)| {
                ScopedRole::new(
                    *role,
                    branch.map(BranchId::new),
                    DateTime::<Utc>::UNIX_EPOCH,
                    None,
                )
            })
            .collect(),
        None,
    )
}

fn request(payload: serde_json::Value) -> ApprovalRequest {
    ApprovalRequest {
        id: RequestId::new("00000000-0000-0000-0000-0000000000bb"),
        organization_id: ORG.to_owned(),
        type_name: "CASH_TRANSFER".to_owned(),
        branch_id: Some("B1".to_owned()),
        object_id: "transfer-1".to_owned(),
        object_version_hash: "h".to_owned(),
        policy_version: "1".to_owned(),
        proposed_payload: payload,
        requested_by: "finance-1".to_owned(),
        requested_at: Utc::now(),
        status: "PENDING".to_owned(),
        idempotency_key: "k".to_owned(),
    }
}

fn executive() -> Actor {
    user("owner", &[(Role::Executive, None), (Role::Admin, None)])
}

#[test]
fn within_the_limit_only_a_branch_manager_of_the_source_decides() {
    let p = TransferApprovalPolicy::new();
    let r = request(json!({ "within_limit": true, "from_branch": "B1" }));
    assert_eq!(
        p.may_decide(&user("mgr-b1", &[(Role::BranchManager, Some("B1"))]), &r),
        Ok(())
    );
    for (who, label) in [
        (
            user("mgr-b2", &[(Role::BranchManager, Some("B2"))]),
            "a branch manager of another branch",
        ),
        (executive(), "the executive (ruling Q2)"),
        (
            user("finance-b1", &[(Role::Finance, Some("B1"))]),
            "FINANCE of the source",
        ),
    ] {
        let refused = p.may_decide(&who, &r).expect_err(label);
        assert_eq!(refused.code(), "WITHIN_LIMIT", "{label}");
    }
}

#[test]
fn above_the_limit_only_the_executive_decides() {
    let p = TransferApprovalPolicy::new();
    for payload in [
        json!({ "within_limit": false, "from_branch": "B1" }),
        json!({ "from_branch": "B1" }),
    ] {
        let r = request(payload.clone());
        assert_eq!(p.may_decide(&executive(), &r), Ok(()), "{payload}");
        let refused = p
            .may_decide(&user("mgr-b1", &[(Role::BranchManager, Some("B1"))]), &r)
            .expect_err("a branch manager above the limit");
        assert_eq!(refused.code(), "ABOVE_LIMIT", "{payload}");
    }
}

/// Above the limit the rule is EXECUTIVE, not EXECUTIVE + ADMIN: an organisation-wide EXECUTIVE with no ADMIN
/// decides (rule-A review note, issuecomment-6116472666).
#[test]
fn above_the_limit_an_executive_without_admin_decides() {
    let p = TransferApprovalPolicy::new();
    let r = request(json!({ "within_limit": false, "from_branch": "B1" }));
    assert_eq!(
        p.may_decide(&user("exec-1", &[(Role::Executive, None)]), &r),
        Ok(())
    );
}

#[test]
fn nobody_closes_their_own_transfer_request() {
    let p = TransferApprovalPolicy::new();
    for who in [
        executive(),
        user("mgr-b1", &[(Role::BranchManager, Some("B1"))]),
    ] {
        assert!(!p.allows_self_decision(&who), "{}", who.user_id);
    }
}
