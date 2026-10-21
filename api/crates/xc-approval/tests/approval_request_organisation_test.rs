//! Ticket #4023 — an approval request records the organisation of whoever asked, and only a decider of
//! that organisation may decide it. Written before the implementation by the ticket's controller
//! (claude/studio-claude-7). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What this file pins (behaviour table §4 of #4023, rows 1–4; row 5, the view, is
//! `api/tests/approval_requests_org_writer_test.rs`):
//! - Row 1: an organisation-wide request (branch NULL) by organisation B's executive is stored with
//!   `organization_id = B`. Before #4023 the insert never wrote the column, so it took the default
//!   organisation.
//! - Row 2: organisation A's executive cannot decide that request: `outOfScope`, the request stays
//!   PENDING, no decision row. Before #4023 the organisation was checked only for branch-bound requests.
//! - Row 3 (kept): organisation B's executive, not the requester, decides it as before.
//! - Row 4 (kept, #2368): a branch-bound request of organisation A is refused to organisation B's decider.
//! - Row 6 (lane controller's ruling 2 on #4023, issuecomment-6306292056): an organisation-wide FINANCE of
//!   organisation B stores branch `MAIN` (`requester_branch`, pinned by #1020 ruling 4), which is the default
//!   organisation's branch. Stored as `(B, MAIN)` the composite FK to `branches (organization_id, id)` refuses
//!   it: the request is refused with `databaseError` and nothing is stored. Before #4023 it was stored silently
//!   as the default organisation's. Which branch it should belong to is a follow-up ticket, not this one.
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

/// The seeded default organisation (`20261011100000_organizations.sql`).
const ORG_A: &str = ORG;
/// A second organisation, created by `second_organisation`.
const ORG_B: &str = "00000000-0000-0000-0000-000000000099";

/// A user of `organization` holding each `(role, branch)`; `None` is organisation-wide.
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

/// An executive of `organization`: EXECUTIVE + ADMIN, organisation-wide, so the request stores no branch.
fn executive(id: &str, organization: &str) -> Actor {
    user(
        id,
        organization,
        &[(Role::Executive, None), (Role::Admin, None)],
    )
}

async fn second_organisation(db: &TestDb) {
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1::uuid, 'ORG2', 'Asia/Bangkok', now())",
    )
    .bind(ORG_B)
    .execute(&db.pool)
    .await
    .expect("the second organisation");
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

/// `who` approves `request`; commits on success, rolls back on a refusal.
async fn approve(db: &TestDb, request: &ApprovalRequest, who: &Actor) -> Result<(), &'static str> {
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A)));
    let mut tx = db.begin().await;
    let got = svc
        .decide(
            &mut tx,
            &request.id,
            who,
            Outcome::Approved,
            "checked",
            &key(),
        )
        .await;
    match got {
        Ok(()) => {
            tx.commit().await.expect("commit");
            Ok(())
        }
        Err(e) => {
            tx.rollback().await.expect("rollback");
            Err(e.code())
        }
    }
}

/// `(branch_id, organization_id)` as stored.
async fn stored(db: &TestDb, request: &ApprovalRequest) -> (Option<String>, String) {
    sqlx::query_as(
        "SELECT branch_id, organization_id::text FROM approval_requests WHERE id = $1::uuid",
    )
    .bind(request.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("the stored request")
}

#[tokio::test]
async fn row1_an_organisation_wide_request_stores_the_requesters_organisation() {
    let db = TestDb::start().await;
    second_organisation(&db).await;
    let r = open(&db, &executive("exec-b", ORG_B), "obj-1").await;
    assert_eq!(
        stored(&db, &r).await,
        (None, ORG_B.to_owned()),
        "organisation B's executive asked, so the request is organisation B's"
    );
    db.finish().await;
}

#[tokio::test]
async fn row2_another_organisations_executive_cannot_decide_an_organisation_wide_request() {
    let db = TestDb::start().await;
    second_organisation(&db).await;
    let r = open(&db, &executive("exec-b", ORG_B), "obj-1").await;
    assert_eq!(
        approve(&db, &r, &executive("exec-a", ORG_A)).await,
        Err("outOfScope"),
        "organisation A's executive decided organisation B's request"
    );
    assert_eq!(db.status(r.id.as_str()).await, "PENDING");
    assert_eq!(db.decisions(r.id.as_str()).await, Vec::new());
    db.finish().await;
}

#[tokio::test]
async fn row3_the_same_organisations_executive_decides_an_organisation_wide_request() {
    let db = TestDb::start().await;
    second_organisation(&db).await;
    let r = open(&db, &executive("exec-b", ORG_B), "obj-1").await;
    assert_eq!(approve(&db, &r, &executive("exec-b2", ORG_B)).await, Ok(()));
    assert_eq!(db.status(r.id.as_str()).await, "APPROVED");
    let decisions = db.decisions(r.id.as_str()).await;
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    assert_eq!(decisions[0].0, "exec-b2");
    db.finish().await;
}

#[tokio::test]
async fn row4_a_branch_bound_request_is_refused_to_another_organisations_decider() {
    let db = TestDb::start().await;
    second_organisation(&db).await;
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id) VALUES ('A-B1', 'A-B1', $1::uuid)",
    )
    .bind(ORG_A)
    .execute(&db.pool)
    .await
    .expect("a branch of organisation A");
    let manager = user("manager-a", ORG_A, &[(Role::BranchManager, Some("A-B1"))]);
    let r = open(&db, &manager, "obj-1").await;
    assert_eq!(stored(&db, &r).await.0.as_deref(), Some("A-B1"));
    assert_eq!(
        approve(&db, &r, &executive("exec-b", ORG_B)).await,
        Err("outOfScope"),
        "organisation B's executive decided a branch-bound request of organisation A"
    );
    assert_eq!(db.status(r.id.as_str()).await, "PENDING");
    assert_eq!(db.decisions(r.id.as_str()).await, Vec::new());
    db.finish().await;
}

#[tokio::test]
async fn row6_an_organisation_wide_finance_of_another_organisation_is_refused_and_nothing_is_stored()
 {
    let db = TestDb::start().await;
    second_organisation(&db).await;
    let finance = user("finance-b", ORG_B, &[(Role::Finance, None)]);
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A)));
    let mut tx = db.begin().await;
    let got = svc
        .request(
            &mut tx,
            TYPE_A,
            "obj-1",
            &json!({ "n": 1 }),
            &finance,
            &key(),
        )
        .await;
    tx.rollback().await.expect("rollback");
    assert_eq!(
        code(got),
        "databaseError",
        "organisation B's request on the default organisation's MAIN was stored"
    );
    assert_eq!(db.count("approval_requests").await, 0);
    db.finish().await;
}
