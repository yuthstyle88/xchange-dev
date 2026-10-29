//! AP-1's audit row carries the organisation of its request (lane controller's ruling A on #5279, 2026-10-29).
//! Written before the fix by the sub-controller claude/2ca5874b. Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! Root cause: `PgAuditLog::append` (`xc-approval/src/infrastructure/audit_log.rs:31-36`) writes no
//! `organization_id`, so every audit row takes the column default, the seeded organisation
//! (`20261011100000_organizations.sql:47`). The same migration adds `(organization_id, branch_id) → branches` to
//! every table with a `branch_id` (`:66`). So when a request of organisation B on B's branch `B-B1` is decided, the
//! audit row `(default organisation, B-B1)` breaks that foreign key, and the decision answers `internalError`.
//!
//! What this file pins: for each of AP-1's three decisions (`decide`, `decide_self`, `decide_by_policy`), a request
//! of organisation B filed at `B-B1` is decided, and its audit row is organisation B's with branch `B-B1`. The
//! requester is a TELLER of `B-B1`, so the request is `(B, B-B1)` with or without #5279.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.

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

/// A second organisation.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
const B_B1: &str = "B-B1";

fn user(id: &str, organization: &str, roles: &[(Role, Option<&str>)]) -> Actor {
    Actor::new(
        id,
        organization,
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

/// Organisation B and its branch `B-B1`.
async fn organisation_b(db: &TestDb) {
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1::uuid, 'ORG2', 'Asia/Bangkok', now())",
    )
    .bind(ORG_B)
    .execute(&db.pool)
    .await
    .expect("the second organisation");
    sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
        .bind(B_B1)
        .bind(ORG_B)
        .execute(&db.pool)
        .await
        .expect("a branch of organisation B");
}

fn anyone(
    subject: &Arc<FakeSubject>,
    self_decision: bool,
) -> Vec<(Arc<dyn ApprovalSubject>, Arc<dyn ApprovalPolicy>)> {
    let subject: Arc<dyn ApprovalSubject> = subject.clone();
    let policy: Arc<dyn ApprovalPolicy> = Arc::new(AnyonePolicy { self_decision });
    vec![(subject, policy)]
}

fn teller_b() -> Actor {
    user("teller-b", ORG_B, &[(Role::Teller, Some(B_B1))])
}

/// The teller of `B-B1` asks; the request is organisation B's, at `B-B1`.
async fn open(db: &TestDb, self_decision: bool) -> ApprovalRequest {
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A), self_decision));
    let mut tx = db.begin().await;
    let request = svc
        .request(
            &mut tx,
            TYPE_A,
            "obj-1",
            Some(&BranchId::new(B_B1)),
            &json!({ "n": 1 }),
            &teller_b(),
            &key(),
        )
        .await
        .unwrap_or_else(|e| panic!("the request opens: {}", e.code()));
    tx.commit().await.expect("commit");
    assert_eq!(
        request.branch_id.as_deref(),
        Some(B_B1),
        "the request is B-B1's"
    );
    request
}

/// `(organization_id, branch_id, action)` of every audit row whose object is `obj-1`.
async fn audit_rows(db: &TestDb) -> Vec<(String, Option<String>, String)> {
    sqlx::query_as(
        "SELECT organization_id::text, branch_id, action FROM audit_events \
         WHERE object_ref = 'obj-1' ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("the audit rows")
}

fn approved_in_b() -> Vec<(String, Option<String>, String)> {
    vec![(
        ORG_B.to_owned(),
        Some(B_B1.to_owned()),
        "approval.approved".to_owned(),
    )]
}

/// `decide`: organisation B's executive approves B's request.
#[tokio::test]
async fn a_person_deciding_another_organisations_branch_request_writes_that_organisations_audit_row()
 {
    let db = TestDb::start().await;
    organisation_b(&db).await;
    let request = open(&db, false).await;
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A), false));
    let executive = user(
        "exec-b",
        ORG_B,
        &[(Role::Executive, None), (Role::Admin, None)],
    );
    let mut tx = db.begin().await;
    let got = svc
        .decide(
            &mut tx,
            &request.id,
            &executive,
            Outcome::Approved,
            "checked",
            &key(),
        )
        .await;
    match got {
        Ok(()) => tx.commit().await.expect("commit"),
        Err(e) => {
            tx.rollback().await.expect("rollback");
            panic!("B's executive decides B's request: {}", e.code());
        }
    }
    assert_eq!(db.status(request.id.as_str()).await, "APPROVED");
    assert_eq!(audit_rows(&db).await, approved_in_b());
    db.finish().await;
}

/// `decide_self`: the proposer closes their own request where the policy allows it (A5).
#[tokio::test]
async fn a_self_decision_in_another_organisation_writes_that_organisations_audit_row() {
    let db = TestDb::start().await;
    organisation_b(&db).await;
    let request = open(&db, true).await;
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A), true));
    let mut tx = db.begin().await;
    let got = svc
        .decide_self(&mut tx, &request.id, &teller_b(), "mine")
        .await;
    match got {
        Ok(()) => tx.commit().await.expect("commit"),
        Err(e) => {
            tx.rollback().await.expect("rollback");
            panic!("the proposer closes B's request: {}", e.code());
        }
    }
    assert_eq!(db.status(request.id.as_str()).await, "APPROVED");
    assert_eq!(audit_rows(&db).await, approved_in_b());
    db.finish().await;
}

/// `decide_by_policy`: the policy closes a request in bound.
#[tokio::test]
async fn a_policy_decision_in_another_organisation_writes_that_organisations_audit_row() {
    let db = TestDb::start().await;
    organisation_b(&db).await;
    let request = open(&db, false).await;
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A), false));
    let mut tx = db.begin().await;
    let got = svc
        .decide_by_policy(&mut tx, &request.id, POLICY_VERSION)
        .await;
    match got {
        Ok(()) => tx.commit().await.expect("commit"),
        Err(e) => {
            tx.rollback().await.expect("rollback");
            panic!("the policy closes B's request: {}", e.code());
        }
    }
    assert_eq!(db.status(request.id.as_str()).await, "APPROVED");
    assert_eq!(audit_rows(&db).await, approved_in_b());
    db.finish().await;
}

/// Unchanged: a request of the seeded organisation still writes the seeded organisation's audit row.
#[tokio::test]
async fn a_decision_in_the_seeded_organisation_still_writes_its_audit_row() {
    let db = TestDb::start().await;
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A), false));
    let mut tx = db.begin().await;
    let request = svc
        .request(
            &mut tx,
            TYPE_A,
            "obj-1",
            Some(&BranchId::new("MAIN")),
            &json!({ "n": 1 }),
            &rates("rates-1"),
            &key(),
        )
        .await
        .unwrap_or_else(|e| panic!("the request opens: {}", e.code()));
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    svc.decide(
        &mut tx,
        &request.id,
        &owner("owner-1"),
        Outcome::Approved,
        "checked",
        &key(),
    )
    .await
    .unwrap_or_else(|e| panic!("the owner decides: {}", e.code()));
    tx.commit().await.expect("commit");
    assert_eq!(
        audit_rows(&db).await,
        vec![(
            ORG.to_owned(),
            Some("MAIN".to_owned()),
            "approval.approved".to_owned()
        )]
    );
    db.finish().await;
}
