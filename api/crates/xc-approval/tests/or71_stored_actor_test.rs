//! Ticket #1020 (OR-7.1) — what the approval core stores about the actor once it takes `access::Actor`
//! (design `docs/design/blueprint/org-roles-branches.md` §3.3; lane controller's rulings 4 and 5 on #1020,
//! issuecomment-6115406683). Written before the implementation by the ticket author
//! (claude/89bd61d9, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What this test pins ([required]):
//! - **Ruling 4 (no stored value changes):** `request()` stores the requester's branch as `legacy_actor`
//!   derives it: NULL for the v3 owner's user (EXECUTIVE + ADMIN, organisation-wide), otherwise the branch of the
//!   requester's role, and `MAIN` for an organisation-wide role of anybody else. `requested_by` is the user's id.
//! - **Ruling 5 (named change):** the decision row's `approver_role` and the audit row's `actor_role` are the
//!   Blueprint role that qualified, spelled as the Blueprint spells it: a BRANCH_MANAGER's decision stores
//!   `"BRANCH_MANAGER"` (before #1020, `"MANAGER"`), a RATE_MANAGER's `"RATE_MANAGER"` (before, `"RATES"`).
//!   The owner's `"EXECUTIVE"` is pinned in `approval_rules_test.rs`.
//!
//! The policy is `AnyonePolicy`, so only the core's rules act. Needs `TEST_DATABASE_URL` (see `common/mod.rs`);
//! nothing here skips.

// `common` is shared with the AP-1 tests; this file uses part of it.
#[allow(unused_imports)]
mod common;

use chrono::{DateTime, Utc};
use common::*;
use serde_json::json;
use std::sync::Arc;

use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;

/// A user holding each `(role, branch)`; `None` is organisation-wide.
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

/// `TYPE_A` with a fake subject and `AnyonePolicy`.
fn anyone(subject: &Arc<FakeSubject>) -> Vec<(Arc<dyn ApprovalSubject>, Arc<dyn ApprovalPolicy>)> {
    let subject: Arc<dyn ApprovalSubject> = subject.clone();
    let policy: Arc<dyn ApprovalPolicy> = Arc::new(AnyonePolicy {
        self_decision: false,
    });
    vec![(subject, policy)]
}

async fn open(db: &TestDb, by: &Actor, object_id: &str) -> ApprovalRequest {
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A)));
    let mut tx = db.begin().await;
    let request = svc
        .request(&mut tx, TYPE_A, object_id, &json!({ "n": 1 }), by, &key())
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit");
    request
}

/// `(branch_id, requested_by)` as stored.
async fn stored(db: &TestDb, request: &ApprovalRequest) -> (Option<String>, String) {
    sqlx::query_as("SELECT branch_id, requested_by FROM approval_requests WHERE id = $1::uuid")
        .bind(request.id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("the stored request")
}

#[tokio::test]
async fn the_owners_request_stores_no_branch() {
    let db = TestDb::start().await;
    let r = open(&db, &owner("owner-1"), "obj-1").await;
    assert_eq!(stored(&db, &r).await, (None, "owner-1".to_owned()));
    db.finish().await;
}

#[tokio::test]
async fn an_employees_request_stores_the_branch_of_their_role() {
    let db = TestDb::start().await;
    for branch in ["B01", "B1", "B2"] {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(branch)
            .execute(&db.pool)
            .await
            .expect("a branch");
    }
    for (who, object, branch) in [
        (
            user("rates-b01", &[(Role::RateManager, Some("B01"))]),
            "obj-1",
            "B01",
        ),
        (
            user("teller-b1", &[(Role::Teller, Some("B1"))]),
            "obj-2",
            "B1",
        ),
        (
            user("finance-b2", &[(Role::Finance, Some("B2"))]),
            "obj-3",
            "B2",
        ),
    ] {
        let r = open(&db, &who, object).await;
        assert_eq!(
            stored(&db, &r).await,
            (Some(branch.to_owned()), who.user_id.clone()),
            "{}",
            who.user_id
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn an_organisation_wide_role_of_anybody_but_the_owner_stores_main() {
    let db = TestDb::start().await;
    let finance = user("finance-org", &[(Role::Finance, None)]);
    let r = open(&db, &finance, "obj-1").await;
    assert_eq!(
        stored(&db, &r).await,
        (Some("MAIN".to_owned()), "finance-org".to_owned())
    );
    db.finish().await;
}

#[tokio::test]
async fn a_decision_stores_the_blueprint_role_that_decided() {
    let db = TestDb::start().await;
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A)));
    for (decider, object, role) in [
        (
            user("manager-1", &[(Role::BranchManager, Some("MAIN"))]),
            "obj-1",
            "BRANCH_MANAGER",
        ),
        (
            user("rates-2", &[(Role::RateManager, Some("MAIN"))]),
            "obj-2",
            "RATE_MANAGER",
        ),
    ] {
        let mut tx = db.begin().await;
        let request = svc
            .request(
                &mut tx,
                TYPE_A,
                object,
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
            &decider,
            Outcome::Approved,
            "checked",
            &key(),
        )
        .await
        .unwrap_or_else(|e| panic!("{role} decides: {}", e.code()));
        tx.commit().await.expect("commit");
        assert_eq!(
            db.decisions(request.id.as_str()).await,
            vec![(
                decider.user_id.clone(),
                role.to_owned(),
                "APPROVED".to_owned(),
                "checked".to_owned()
            )]
        );
        let audit = db.audit_by(&decider.user_id).await;
        assert!(
            !audit.is_empty() && audit.iter().all(|(r, _)| r == role),
            "the audit row's role is {role}: {audit:?}"
        );
    }
    db.finish().await;
}
