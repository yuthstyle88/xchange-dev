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

use chrono::Utc;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
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
    for role in [Role::Manager, Role::Backend, Role::Finance] {
        let manager = Actor::staff(
            format!("mgr-{}", role.as_str()),
            role,
            BranchId::new("MAIN"),
        )
        .unwrap();
        let refused = policy
            .may_decide(&manager, &request())
            .expect_err("a manager of the branch is still refused");
        assert_eq!(refused.code(), "outOfScope", "{}", role.as_str());
        assert!(
            !policy.allows_self_decision(&manager),
            "{}: no self-decision",
            role.as_str()
        );
    }
    assert_eq!(
        policy.may_decide(&Actor::owner("owner-1"), &request()),
        Ok(()),
        "the owner decides, as before"
    );
}
