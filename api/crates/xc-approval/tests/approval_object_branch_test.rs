//! #5279 site 2 — an approval request is filed under the branch of its object, whoever asks (owner, #5279
//! issuecomment-6452390376: *"เก็บที่สาขาของสิ่งที่ขอ"*; design `docs/design/org-wide-actor-branch.md` §0, §3, §5
//! rows S2-2 … S2-4). Written before the fix by the sub-controller claude/2ca5874b. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Each row asks with a requester whose rows would give a different branch under the old `requester_branch`
//! (`approval_service.rs:9-49`), so a row cannot pass by that rule. S2-1 (an organisation-wide FINANCE on `B01`)
//! is `or71_stored_actor_test::an_organisation_wide_role_files_the_request_at_the_objects_branch`; S2-5 (another
//! organisation) is `approval_request_organisation_test::row6_…`.
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
use xc_approval::domain::request::ApprovalRequest;
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;

/// A user of the default organisation holding each `(role, branch)`; `None` is organisation-wide.
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

async fn branches(db: &TestDb) {
    for branch in ["B01", "B02"] {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(branch)
            .execute(&db.pool)
            .await
            .expect("a branch");
    }
}

/// `by` asks for an approval of `object_id`, whose branch is `branch`.
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
        .unwrap_or_else(|e| panic!("the request opens: {}", e.code()));
    tx.commit().await.expect("commit");
    request
}

/// `branch_id` as stored.
async fn stored_branch(db: &TestDb, request: &ApprovalRequest) -> Option<String> {
    sqlx::query_scalar("SELECT branch_id FROM approval_requests WHERE id = $1::uuid")
        .bind(request.id.as_str())
        .fetch_one(&db.pool)
        .await
        .expect("the stored request")
}

/// S2-2: a RATE_MANAGER of `B01` and `B02` asking about an object of `B02` files it at `B02`. The old rule took
/// the first role row, `B01`.
#[tokio::test]
async fn a_two_branch_requester_files_at_the_objects_branch_not_its_first_row() {
    let db = TestDb::start().await;
    branches(&db).await;
    let rates = user(
        "rates-2b",
        &[
            (Role::RateManager, Some("B01")),
            (Role::RateManager, Some("B02")),
        ],
    );
    let r = open(&db, &rates, "obj-1", Some("B02")).await;
    assert_eq!(r.branch_id.as_deref(), Some("B02"), "the answered request");
    assert_eq!(
        stored_branch(&db, &r).await.as_deref(),
        Some("B02"),
        "the stored row"
    );
    db.finish().await;
}

/// S2-3: a TELLER of `B01` asking about an object of the organisation (no branch) files it with no branch. The
/// old rule filed it at the teller's `B01`.
#[tokio::test]
async fn an_object_of_the_organisation_is_filed_with_no_branch_whoever_asks() {
    let db = TestDb::start().await;
    branches(&db).await;
    let teller = user("teller-b01", &[(Role::Teller, Some("B01"))]);
    let r = open(&db, &teller, "obj-1", None).await;
    assert_eq!(r.branch_id, None, "the answered request");
    assert_eq!(stored_branch(&db, &r).await, None, "the stored row");
    db.finish().await;
}

/// S2-4: the owner (EXECUTIVE + ADMIN, organisation-wide) asking about an object of `B01` files it at `B01`. The
/// old rule filed the owner's requests with no branch.
#[tokio::test]
async fn the_owners_request_on_a_branch_object_is_filed_at_that_branch() {
    let db = TestDb::start().await;
    branches(&db).await;
    let r = open(&db, &owner("owner-1"), "obj-1", Some("B01")).await;
    assert_eq!(r.branch_id.as_deref(), Some("B01"), "the answered request");
    assert_eq!(
        stored_branch(&db, &r).await.as_deref(),
        Some("B01"),
        "the stored row"
    );
    db.finish().await;
}

/// S2-1 for the answered value too: an organisation-wide FINANCE (the old rule: `MAIN`) asking about an object of
/// `B02` gets back a request of `B02`.
#[tokio::test]
async fn an_organisation_wide_requester_gets_back_a_request_of_the_objects_branch() {
    let db = TestDb::start().await;
    branches(&db).await;
    let finance = user("finance-org", &[(Role::Finance, None)]);
    let r = open(&db, &finance, "obj-1", Some("B02")).await;
    assert_eq!(r.branch_id.as_deref(), Some("B02"));
    db.finish().await;
}
