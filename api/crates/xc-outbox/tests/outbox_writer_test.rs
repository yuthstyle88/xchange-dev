//! OB-1 up-front tests — `OutboxWriter::enqueue_in` (design broker-seam.md §5.1, §5.2, §11
//! "Up-front tests per ticket: OB-1"). Written before the implementation by the ticket author.
//! Do not edit, delete or weaken; add tests in a file of your own.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn the_same_key_with_the_same_payload_enqueues_once_and_answers_the_same_id() {
    let db = TestDb::start().await;
    let k = key();
    let first = db
        .enqueue(&EARN, "tx-1", &k, json!({"transactionId": "tx-1"}))
        .await;
    let second = db
        .enqueue(&EARN, "tx-1", &k, json!({"transactionId": "tx-1"}))
        .await;
    assert_eq!(first, second, "a repeat answers the stored id");
    assert_eq!(db.count().await, 1, "a repeat writes no second row");
    db.finish().await;
}

#[tokio::test]
async fn the_same_key_with_another_payload_is_an_idempotency_conflict_and_writes_nothing() {
    let db = TestDb::start().await;
    let k = key();
    let id = db
        .enqueue(&EARN, "tx-1", &k, json!({"transactionId": "tx-1"}))
        .await;
    let before = db.row(&id).await;

    let mut tx = db.begin().await;
    let out = OutboxWriter::enqueue_in(
        &mut tx,
        ORG,
        &EARN,
        "tx-1",
        &k,
        &json!({"transactionId": "tx-2"}),
    )
    .await;
    assert!(
        is_idempotency_conflict(&out),
        "expected idempotencyConflict, got {out:?}"
    );
    drop(tx);

    assert_eq!(db.count().await, 1);
    let after = db.row(&id).await;
    assert_eq!(
        after.payload_hash, before.payload_hash,
        "the stored row is unchanged"
    );
    assert_eq!(after.payload_redacted, before.payload_redacted);
    db.finish().await;
}

#[tokio::test]
async fn object_key_order_does_not_change_the_hash() {
    let db = TestDb::start().await;
    let k = key();
    let first = db
        .enqueue(&EARN, "tx-1", &k, json!({"a": 1, "b": {"x": 1, "y": 2}}))
        .await;
    let second = db
        .enqueue(&EARN, "tx-1", &k, json!({"b": {"y": 2, "x": 1}, "a": 1}))
        .await;
    assert_eq!(
        first, second,
        "the same content in another key order is the same payload"
    );
    assert_eq!(db.count().await, 1);
    db.finish().await;
}

#[tokio::test]
async fn the_hash_is_of_the_unredacted_payload() {
    // Two payloads that redact to the same text (`***5678`) are still two payloads: a key reused
    // with another phone is a conflict, not a silent repeat.
    let db = TestDb::start().await;
    let k = key();
    db.enqueue(
        &EARN,
        "tx-1",
        &k,
        json!({"captureId": "c-1", "phone": "0811115678"}),
    )
    .await;

    let mut tx = db.begin().await;
    let out = OutboxWriter::enqueue_in(
        &mut tx,
        ORG,
        &EARN,
        "tx-1",
        &k,
        &json!({"captureId": "c-1", "phone": "0822225678"}),
    )
    .await;
    assert!(
        is_idempotency_conflict(&out),
        "expected idempotencyConflict, got {out:?}"
    );
    drop(tx);
    db.finish().await;
}

#[tokio::test]
async fn the_same_key_in_two_organisations_is_two_rows() {
    let db = TestDb::start().await;
    db.second_org().await;
    let k = key();
    let payload = json!({"transactionId": "tx-1"});

    let mut tx = db.begin().await;
    let a = OutboxWriter::enqueue_in(&mut tx, ORG, &EARN, "tx-1", &k, &payload)
        .await
        .expect("org A");
    let b = OutboxWriter::enqueue_in(&mut tx, ORG_B, &EARN, "tx-1", &k, &payload)
        .await
        .expect("org B: UNIQUE includes organization_id");
    tx.commit().await.expect("commit");

    assert_ne!(a, b);
    assert_eq!(db.count().await, 2);
    assert_eq!(db.row(&a).await.organization_id, ORG);
    assert_eq!(db.row(&b).await.organization_id, ORG_B);
    db.finish().await;
}

#[tokio::test]
async fn the_same_key_for_two_event_types_is_two_rows() {
    let db = TestDb::start().await;
    let k = key();
    let a = db
        .enqueue(&EARN, "tx-1", &k, json!({"transactionId": "tx-1"}))
        .await;
    let b = db
        .enqueue(&GL, "tx-1", &k, json!({"transactionId": "tx-1"}))
        .await;
    assert_ne!(
        a, b,
        "UNIQUE is (organization_id, event_type, idempotency_key)"
    );
    assert_eq!(db.count().await, 2);
    db.finish().await;
}

#[tokio::test]
async fn fan_out_two_kinds_in_one_transaction_gives_two_rows_one_per_destination() {
    let db = TestDb::start().await;
    let mut tx = db.begin().await;
    let payload = json!({"transactionId": "tx-9"});
    let earn = OutboxWriter::enqueue_in(&mut tx, ORG, &EARN, "tx-9", "earn:tx-9", &payload)
        .await
        .expect("loyalty row");
    let gl = OutboxWriter::enqueue_in(&mut tx, ORG, &GL, "tx-9", "gl:tx-9", &payload)
        .await
        .expect("accountzing row");
    tx.commit().await.expect("commit");

    let earn = db.row(&earn).await;
    let gl = db.row(&gl).await;
    assert_eq!(
        (
            earn.event_type.as_str(),
            earn.destination.as_str(),
            earn.aggregate_type.as_str()
        ),
        ("test.loyalty.earn", "loyalty", "transaction")
    );
    assert_eq!(
        (
            gl.event_type.as_str(),
            gl.destination.as_str(),
            gl.aggregate_type.as_str()
        ),
        ("test.gl", "accountzing", "transaction")
    );
    assert_eq!(db.count().await, 2);
    db.finish().await;
}

#[tokio::test]
async fn a_rollback_of_the_callers_transaction_writes_no_row() {
    let db = TestDb::start().await;
    let mut tx = db.begin().await;
    let payload = json!({"transactionId": "tx-9"});
    OutboxWriter::enqueue_in(&mut tx, ORG, &EARN, "tx-9", "earn:tx-9", &payload)
        .await
        .expect("loyalty row");
    OutboxWriter::enqueue_in(&mut tx, ORG, &GL, "tx-9", "gl:tx-9", &payload)
        .await
        .expect("accountzing row");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        db.count().await,
        0,
        "the events live and die with the business work"
    );
    db.finish().await;
}

#[tokio::test]
async fn payload_redacted_masks_a_phone_and_drops_a_token() {
    let db = TestDb::start().await;
    let id = db
        .enqueue(
            &EARN,
            "tx-1",
            &key(),
            json!({"captureId": "c-1", "phone": "0812345678", "token": "s3cret"}),
        )
        .await;
    let stored = db.row(&id).await.payload_redacted;
    assert_eq!(stored["captureId"], json!("c-1"), "references are kept");
    assert_eq!(
        stored["phone"],
        json!("***5678"),
        "a phone keeps its last 4 only"
    );
    assert!(
        stored.get("token").is_none(),
        "a token is dropped: {stored}"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_new_row_is_pending_due_now_with_no_attempts() {
    let db = TestDb::start().await;
    let k = key();
    let id = db
        .enqueue(&REVERSE, "tx-3", &k, json!({"transactionId": "tx-3"}))
        .await;
    let row = db.row(&id).await;
    assert_eq!(row.status, "PENDING");
    assert_eq!(row.attempts, 0);
    assert_eq!(row.aggregate_id, "tx-3");
    assert_eq!(row.idempotency_key, k);
    assert_eq!(
        row.payload_hash.len(),
        64,
        "sha256 as hex: {}",
        row.payload_hash
    );
    assert!(!row.delivered);
    assert!(
        row.due_in_secs <= 0.5,
        "due at once, got {} s",
        row.due_in_secs
    );
    db.finish().await;
}
