//! #5630 W-3 — the role an approval decision is stored and audited under (design
//! `docs/design/org-wide-actor-branch.md` §14.5; rulings Q-1 A and Q-2 B on #5661: no grant reaches
//! `ApprovalService`, so it is the caller's `Standing` order in Blueprint names, the owner `EXECUTIVE`).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Today `approver_role` (`approval_service.rs:10-28`) has its own order (EXECUTIVE, FINANCE, BRANCH_MANAGER, …),
//! so a decider holding FINANCE and TELLER is written `FINANCE`; under the one rule it is `TELLER`, the first role
//! of `STAFF_PRIORITY`. Single-role deciders and the owner are unchanged
//! (`or71_stored_actor_test::a_decision_stores_the_blueprint_role_that_decided`, `approval_rules_test`).
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.

#[allow(unused_imports)]
mod common;

use chrono::{DateTime, Utc};
use common::*;
use serde_json::json;
use std::sync::Arc;

use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::Outcome;
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;

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

fn anyone(subject: &Arc<FakeSubject>) -> Vec<(Arc<dyn ApprovalSubject>, Arc<dyn ApprovalPolicy>)> {
    let subject: Arc<dyn ApprovalSubject> = subject.clone();
    let policy: Arc<dyn ApprovalPolicy> = Arc::new(AnyonePolicy {
        self_decision: false,
    });
    vec![(subject, policy)]
}

/// Opens a request by `rates-1` at `MAIN` and has `decider` approve it; the decision's stored role.
async fn decided_as(db: &TestDb, object: &str, decider: &Actor) -> String {
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A)));
    let mut tx = db.begin().await;
    let request = svc
        .request(
            &mut tx,
            TYPE_A,
            object,
            Some(&BranchId::new("MAIN")),
            &json!({ "n": 1 }),
            &rates("rates-1"),
            &key(),
        )
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    svc.decide(
        &mut tx,
        &request.id,
        decider,
        Outcome::Approved,
        "checked",
        &key(),
    )
    .await
    .unwrap_or_else(|e| panic!("{} decides: {}", decider.user_id, e.code()));
    tx.commit().await.expect("commit");
    let decisions = db.decisions(request.id.as_str()).await;
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    decisions[0].1.clone()
}

/// D-1: FINANCE + TELLER at `MAIN` ⇒ `TELLER` (the `Standing` order), on the decision and the audit row.
#[tokio::test]
async fn d1_a_finance_teller_decides_as_teller() {
    let db = TestDb::start().await;
    let decider = user(
        "fin-teller",
        &[(Role::Finance, Some("MAIN")), (Role::Teller, Some("MAIN"))],
    );
    assert_eq!(decided_as(&db, "obj-d1", &decider).await, "TELLER");
    let audit = db.audit_by("fin-teller").await;
    assert!(
        !audit.is_empty() && audit.iter().all(|(r, _)| r == "TELLER"),
        "D-1: {audit:?}"
    );
    db.finish().await;
}

/// D-2: the owner (EXECUTIVE + ADMIN) ⇒ `EXECUTIVE`, unchanged.
#[tokio::test]
async fn d2_the_owner_decides_as_executive() {
    let db = TestDb::start().await;
    let owner = user("owner-d2", &[(Role::Executive, None), (Role::Admin, None)]);
    assert_eq!(decided_as(&db, "obj-d2", &owner).await, "EXECUTIVE");
    db.finish().await;
}

/// D-3: an organisation-wide ADMIN + VERIFICATION_REVIEWER ⇒ `ADMIN` (ADMIN precedes VERIFICATION_REVIEWER in the
/// `Standing` order; today's list also says `ADMIN`).
#[tokio::test]
async fn d3_an_admin_reviewer_decides_as_admin() {
    let db = TestDb::start().await;
    let decider = user(
        "admin-vr",
        &[(Role::VerificationReviewer, None), (Role::Admin, None)],
    );
    assert_eq!(decided_as(&db, "obj-d3", &decider).await, "ADMIN");
    db.finish().await;
}
