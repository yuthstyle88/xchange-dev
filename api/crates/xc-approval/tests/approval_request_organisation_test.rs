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
//! - Row 6, restated by #5279 (owner, #5279 issuecomment-6452390376: *"เก็บที่สาขาของสิ่งที่ขอ"*; design
//!   `docs/design/org-wide-actor-branch.md` §3, S2-5; restated by the sub-controller claude/2ca5874b). It replaces
//!   decision C (#4094) and the lane controller's ruling 2 on #4023 (issuecomment-6306292056). An
//!   organisation-wide FINANCE of organisation B asking on an object of B's branch `B-B1` is stored as
//!   `(B, B-B1)`. Before #5279 `requester_branch` filed it at the default
//!   organisation's `MAIN`, and the composite FK refused it with `databaseError`.
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

async fn open(db: &TestDb, by: &Actor, object_id: &str, branch: Option<&str>) -> ApprovalRequest {
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A)));
    let branch = branch.map(BranchId::new);
    let mut tx = db.begin().await;
    let request = svc
        .request(
            &mut tx,
            TYPE_A,
            object_id,
            branch.as_ref(),
            &json!({ "n": 1 }),
            by,
            &key(),
        )
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
    let r = open(&db, &executive("exec-b", ORG_B), "obj-1", None).await;
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
    let r = open(&db, &executive("exec-b", ORG_B), "obj-1", None).await;
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
    let r = open(&db, &executive("exec-b", ORG_B), "obj-1", None).await;
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
    let r = open(&db, &manager, "obj-1", Some("A-B1")).await;
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

/// #5279 S2-5, restated from `row6_an_organisation_wide_finance_of_another_organisation_is_refused_and_nothing_is_stored`.
#[tokio::test]
async fn row6_an_organisation_wide_finance_of_another_organisation_files_at_its_objects_branch() {
    let db = TestDb::start().await;
    second_organisation(&db).await;
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id) VALUES ('B-B1', 'B-B1', $1::uuid)",
    )
    .bind(ORG_B)
    .execute(&db.pool)
    .await
    .expect("a branch of organisation B");
    let finance = user("finance-b", ORG_B, &[(Role::Finance, None)]);
    let svc = db.service(anyone(&FakeSubject::new(TYPE_A)));
    let mut tx = db.begin().await;
    let got = svc
        .request(
            &mut tx,
            TYPE_A,
            "obj-1",
            Some(&BranchId::new("B-B1")),
            &json!({ "n": 1 }),
            &finance,
            &key(),
        )
        .await;
    let r = match got {
        Ok(r) => r,
        Err(e) => {
            tx.rollback().await.expect("rollback");
            panic!(
                "organisation B's request on its own branch was refused: {}",
                e.code()
            );
        }
    };
    tx.commit().await.expect("commit");
    assert_eq!(
        stored(&db, &r).await,
        (Some("B-B1".to_owned()), ORG_B.to_owned()),
        "the request belongs to the object's branch, in organisation B"
    );
    // Deciding it is not pinned here: AP-1's audit row is written without the organisation
    // (`xc-approval/src/infrastructure/audit_log.rs:31-36`), so a decision on another organisation's branch-bound
    // request breaks `(organization_id, branch_id) → branches` today, whatever #5279 does. That is its own ticket.
    db.finish().await;
}
