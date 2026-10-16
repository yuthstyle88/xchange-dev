//! A supersede on a capture whose customer's Identity subject is already known must reach Broker: the new ACTIVE row
//! writes its `broker.attribution` outbox row in the same transaction and sets `pushed_at`, exactly as `capture_in`
//! does (design `docs/design/blueprint/broker-seam.md` §4.2: "A capture whose `customer_sub` is known writes its
//! outbox row in the same transaction (§5.3) and sets `pushed_at`"). `on_customer_sub_known` cannot pick it up later:
//! it runs once, when the subject is first linked, and that has already happened.
//!
//! Written before the fix by the ticket author (claude/8389e7f9, on the BE lane controller claude/1656900d's order).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Uses FD-1's harness (`tests/common/mod.rs`). Needs `TEST_DATABASE_URL`; nothing skips.

mod common;

use std::sync::Arc;

use common::{ORG, TestDb, at};
use serde_json::{Value, json};
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_referral::application::capture_service::{
    CaptureError, CaptureInput, ReferralCaptureService,
};
use xc_referral::domain::capture::{CaptureId, CaptureSource, CaptureTarget};
use xc_referral::domain::referral_code::ReferralCode;
use xc_referral::infrastructure::capture_repository::PgCaptureRepository;

fn service(db: &TestDb) -> ReferralCaptureService {
    ReferralCaptureService::new(
        Arc::new(PgCaptureRepository::new()),
        Arc::new(PgAuditLog::new(db.pool.clone())),
    )
}

fn code(text: &str) -> ReferralCode {
    ReferralCode::parse(text).expect("a valid code")
}

fn on_transaction(id: Uuid, text: &str, sub: Option<&str>) -> CaptureInput {
    CaptureInput {
        code: code(text),
        on: CaptureTarget::Transaction(id),
        source: CaptureSource::Desk,
        customer_sub: sub.map(str::to_owned),
        by: "teller-1".to_owned(),
        at: at(0),
    }
}

async fn exchange_of(db: &TestDb, customer: &str) -> Uuid {
    db.seed_customer(customer).await;
    db.seed_transaction(customer).await.0
}

async fn capture(db: &TestDb, input: CaptureInput) -> CaptureId {
    let mut tx = db.begin().await;
    let id = service(db)
        .capture_in(&mut tx, input)
        .await
        .expect("captured");
    tx.commit().await.expect("commit");
    id
}

async fn supersede(db: &TestDb, id: &CaptureId, text: &str) -> Result<CaptureId, CaptureError> {
    let mut tx = db.begin().await;
    let out = service(db)
        .supersede_in(&mut tx, id, code(text), "corrected", "owner-1", at(60))
        .await;
    if out.is_ok() {
        tx.commit().await.expect("commit");
    }
    out
}

async fn row(db: &TestDb, id: &CaptureId) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(r) FROM referral_attributions r WHERE r.id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("the capture row")
}

async fn outbox_rows(db: &TestDb) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('org', organization_id::text, 'event_type', event_type, \
             'destination', destination, 'aggregate_type', aggregate_type, \
             'aggregate_id', aggregate_id, 'key', idempotency_key, \
             'payload', payload_redacted, 'status', status) \
         FROM outbox_events ORDER BY seq",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read outbox_events")
}

fn pushed(id: &CaptureId) -> Value {
    json!({
        "org": ORG,
        "event_type": "broker.attribution",
        "destination": "broker",
        "aggregate_type": "referral_attribution",
        "aggregate_id": id.to_string(),
        "key": format!("attr:{id}"),
        "payload": { "captureId": id.to_string() },
        "status": "PENDING",
    })
}

/// The bug: the superseding row of a capture with a known sub is pushed in the same transaction.
#[tokio::test]
async fn superseding_a_capture_with_a_known_sub_pushes_the_new_row_in_the_same_transaction() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let old = capture(&db, on_transaction(tx_id, "AB12CD", Some("sub-1"))).await;
    assert_eq!(
        outbox_rows(&db).await,
        vec![pushed(&old)],
        "the first capture is pushed"
    );

    let new = supersede(&db, &old, "ZZ99YY").await.expect("superseded");

    let r = row(&db, &new).await;
    assert_eq!(r["customer_sub"], "sub-1");
    assert_eq!(r["code"], "ZZ99YY");
    assert!(
        r["pushed_at"].is_string(),
        "the new row is pushed, so pushed_at is set: {r}"
    );
    assert_eq!(
        outbox_rows(&db).await,
        vec![pushed(&old), pushed(&new)],
        "the correction reaches Broker as its own outbox row"
    );
    db.finish().await;
}

/// Control (unchanged behaviour): a capture with no sub yet is superseded without a push; it waits for
/// `on_customer_sub_known`.
#[tokio::test]
async fn superseding_a_capture_with_no_sub_still_pushes_nothing() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let old = capture(&db, on_transaction(tx_id, "AB12CD", None)).await;

    let new = supersede(&db, &old, "ZZ99YY").await.expect("superseded");

    assert_eq!(row(&db, &new).await["pushed_at"], Value::Null);
    assert_eq!(outbox_rows(&db).await.len(), 0, "nothing pushed yet");
    db.finish().await;
}

/// A refused supersede (empty reason) writes no outbox row, even when the sub is known.
#[tokio::test]
async fn a_refused_supersede_writes_no_outbox_row() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let old = capture(&db, on_transaction(tx_id, "AB12CD", Some("sub-1"))).await;

    let mut tx = db.begin().await;
    let out = service(&db)
        .supersede_in(&mut tx, &old, code("ZZ99YY"), "  ", "owner-1", at(60))
        .await;
    assert!(matches!(out, Err(CaptureError::EmptyReason)), "{out:?}");
    tx.rollback().await.expect("rollback");

    assert_eq!(outbox_rows(&db).await, vec![pushed(&old)]);
    db.finish().await;
}
