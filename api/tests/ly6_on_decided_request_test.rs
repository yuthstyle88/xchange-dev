//! #2881 §3 — the loyalty subject's `on_decided` acts on the request being decided, not on "the
//! latest request for this object".
//!
//! Written before the implementation by the ticket author (claude/studio-claude-4). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Today `rule_approval.rs` looks the request up with `ORDER BY requested_at DESC LIMIT 1`, which
//! is right only while no other request for the object is newer. These tests seed such a newer,
//! already-decided "decoy" request for the same object (it stands in for any second row the
//! lookup could pick: clock skew, a repaired row, a future re-submit), then decide the real one
//! through `ApprovalService::decide` and check what reached the fake Loyalty writer:
//! - APPROVED ⇒ `approvedRef` is the decided request's id;
//! - REJECTED ⇒ the reason sent is the decided request's own reason.

mod loyalty_fake_writer;
mod quote_common;

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::approval_subjects::ApprovalSubjectDeps;
use xchange::presentation::http::v3::commands::make_approval_service_with;

use loyalty_fake_writer::FakeWriter;
use quote_common::{ORG_1, TestDb};

const SUBJECT: &str = "LOYALTY_RULE_VERSION";

fn executive(user: &str) -> Actor {
    Actor::new(
        user,
        ORG_1,
        vec![ScopedRole::new(
            Role::Executive,
            None,
            chrono::DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

/// Opens the real request for the fake's version, committed, and returns its id.
async fn open_request(db: &TestDb, writer: &FakeWriter) -> String {
    let deps = ApprovalSubjectDeps {
        pool: db.pool.clone(),
        loyalty: Some(writer.as_writer()),
    };
    let service = make_approval_service_with(&deps);
    let mut tx = db.pool.begin().await.expect("begin");
    let req = service
        .request(
            &mut tx,
            SUBJECT,
            &writer.object_id(),
            &json!({ "kind": "POINT", "versionId": writer.id.to_string() }),
            &executive("user-EXECUTIVE"),
            &Uuid::new_v4().to_string(),
        )
        .await
        .expect("open the real request");
    tx.commit().await.expect("commit");
    req.id.as_str().to_owned()
}

/// A REJECTED request for the same object, requested an hour later than now, with its decision.
async fn seed_newer_decoy(db: &TestDb, writer: &FakeWriter) -> String {
    let decoy = Uuid::new_v4().to_string();
    db.exec(&format!(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
         policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES ('{decoy}', '{SUBJECT}', NULL, '{object}', 'decoy-hash', '1', '{{}}'::jsonb, \
         'user-DECOY', now() + interval '1 hour', 'REJECTED', 'decoy-{decoy}', '00000000-0000-0000-0000-000000000001')",
        object = writer.object_id(),
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO approval_decisions (id, request_id, approver_id, approver_role, outcome, reason, at) \
         VALUES ('{}', '{decoy}', 'user-DECOY', 'EXECUTIVE', 'REJECTED', 'DECOY-REASON-2881', now() + interval '1 hour')",
        Uuid::new_v4()
    ))
    .await;
    decoy
}

async fn decide(
    db: &TestDb,
    writer: &FakeWriter,
    request_id: &str,
    outcome: Outcome,
    reason: &str,
) {
    let deps = ApprovalSubjectDeps {
        pool: db.pool.clone(),
        loyalty: Some(writer.as_writer()),
    };
    let service = make_approval_service_with(&deps);
    let mut tx = db.pool.begin().await.expect("begin");
    service
        .decide(
            &mut tx,
            &RequestId::new(request_id),
            &executive("user-EXECUTIVE-2"),
            outcome,
            reason,
            &Uuid::new_v4().to_string(),
        )
        .await
        .expect("another executive decides");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn an_approval_sends_the_decided_requests_id_as_approved_ref() {
    let db = TestDb::start().await;
    let writer = FakeWriter::new();
    let real = open_request(&db, &writer).await;
    let decoy = seed_newer_decoy(&db, &writer).await;

    decide(&db, &writer, &real, Outcome::Approved, "ok").await;

    assert_eq!(
        writer.approved_refs(),
        vec![real.clone()],
        "Loyalty's approvedRef must be the request that was decided ({real}), not the newest row for the object ({decoy})"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_rejection_sends_the_decided_requests_own_reason() {
    let db = TestDb::start().await;
    let writer = FakeWriter::new();
    let real = open_request(&db, &writer).await;
    let _decoy = seed_newer_decoy(&db, &writer).await;

    decide(
        &db,
        &writer,
        &real,
        Outcome::Rejected,
        "the real reason 2881",
    )
    .await;

    assert_eq!(
        writer.reject_reasons(),
        vec!["the real reason 2881".to_owned()],
        "the reason sent to Loyalty must be the decided request's own, not the decoy's"
    );
    db.finish().await;
}

#[tokio::test]
async fn without_any_other_request_the_approval_still_sends_its_id() {
    let db = TestDb::start().await;
    let writer = FakeWriter::new();
    let real = open_request(&db, &writer).await;

    decide(&db, &writer, &real, Outcome::Approved, "ok").await;

    assert_eq!(
        writer.approved_refs(),
        vec![real],
        "unchanged: one request, its own id"
    );
    db.finish().await;
}
