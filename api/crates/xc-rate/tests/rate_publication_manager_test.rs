//! Ticket #874 — once a branch manager or FINANCE reaches `approvalDecide`, a rate publication is
//! still the owner's to decide: `RatePublicationPolicy` refuses them `outOfScope` (FINANCE added for
//! RG4 of #682, lane controller 2026-10-10).
//! Written before the implementation by the ticket author (claude/f03345fe). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Lane controller's ruling (claude/608dff, 2026-10-10): BRANCH_MANAGER qualifies for
//! `ApprovalsDecide`, and each type's policy still decides; rates stay OWNER-only through
//! `RatePublicationPolicy` (`crates/xc-rate/src/application/approval/rate_publication.rs:94-102`).
//! This test locks that the ticket changes no policy; it passes before and after.
//!
//! Signatures: `RatePublicationPolicy::new(pool)`, `ApprovalPolicy::may_decide(&Actor,
//! &ApprovalRequest) -> Result<(), Refusal>`, `Refusal::code()`; none new. The policy never touches
//! its pool, so a lazy one that is never connected will do.
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: the policy takes
//! `access::Actor`. MANAGER and BACKEND are both the B1-mapped BRANCH_MANAGER on MAIN, FINANCE is FINANCE, the
//! owner is EXECUTIVE + ADMIN organisation-wide; each keeps its outcome. Ruling 3 on #1020 (EXECUTIVE decides)
//! adds one case: ADMIN alone (the v3 SYSTEM) is refused too, and `an_executive_without_admin_decides_…` pins the
//! ruling's other half: EXECUTIVE alone, with no ADMIN, decides and self-decides (rule-A review must-fix,
//! issuecomment-6116472666).

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
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
use xc_rate::application::approval::rate_publication::RatePublicationPolicy;

fn request() -> ApprovalRequest {
    ApprovalRequest {
        id: RequestId::new("00000000-0000-0000-0000-000000000099"),
        type_name: "RATE_PUBLICATION".to_owned(),
        branch_id: Some("MAIN".to_owned()),
        object_id: "draft-1".to_owned(),
        object_version_hash: "h".to_owned(),
        policy_version: "1".to_owned(),
        proposed_payload: json!({}),
        requested_by: "rates-1".to_owned(),
        requested_at: Utc::now(),
        status: "PENDING".to_owned(),
        idempotency_key: "k".to_owned(),
    }
}

#[tokio::test]
async fn a_branch_manager_may_not_decide_a_rate_publication() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://nobody@127.0.0.1:1/never_connected_test")
        .expect("a lazy pool");
    let policy = RatePublicationPolicy::new(pool);
    for (v3, role) in [
        ("MANAGER", Role::BranchManager),
        ("BACKEND", Role::BranchManager),
        ("FINANCE", Role::Finance),
        ("SYSTEM", Role::Admin),
    ] {
        let manager = user(&format!("mgr-{v3}"), &[(role, Some("MAIN"))]);
        let refused = policy
            .may_decide(&manager, &request())
            .expect_err("a manager of the branch is still refused");
        assert_eq!(refused.code(), "outOfScope", "{v3}");
        assert!(
            !policy.allows_self_decision(&manager),
            "{v3}: no self-decision"
        );
    }
    let owner = user("owner-1", &[(Role::Executive, None), (Role::Admin, None)]);
    assert_eq!(
        policy.may_decide(&owner, &request()),
        Ok(()),
        "the owner decides, as before"
    );
}

/// OR-7.1 #1020 ruling 3: EXECUTIVE decides a rate publication — EXECUTIVE alone, organisation-wide and with **no**
/// ADMIN, decides it and may close their own request (a rule that also asked for ADMIN would lock out every
/// Blueprint EXECUTIVE without an ADMIN grant).
#[tokio::test]
async fn an_executive_without_admin_decides_and_self_decides_a_rate_publication() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://nobody@127.0.0.1:1/never_connected_test")
        .expect("a lazy pool");
    let policy = RatePublicationPolicy::new(pool);
    let executive = user("exec-1", &[(Role::Executive, None)]);
    assert_eq!(
        policy.may_decide(&executive, &request()),
        Ok(()),
        "EXECUTIVE without ADMIN decides"
    );
    assert!(
        policy.allows_self_decision(&executive),
        "EXECUTIVE without ADMIN closes their own request"
    );
}
