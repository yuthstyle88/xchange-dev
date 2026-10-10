//! Regression tests for OB-1 (#899, PR 1016 must-fixes):
//! A worker whose lease has run out must not overwrite a row's final status
//! and must not run on_final a second time when recording a late Retry or Final.

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;
use xc_outbox::domain::store::RecordStep;

#[tokio::test]
async fn late_retry_after_lapsed_lease_leaves_final_status_unchanged_and_does_not_call_on_final() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;

    // Worker A claims attempt 1.
    let claimed_a = db.store().claim_due(10, LEASE).await.expect("claim A");
    assert_eq!(claimed_a.len(), 1);
    assert_eq!(claimed_a[0].id, id);
    assert_eq!(claimed_a[0].attempts, 1);

    // Worker A's lease expires.
    db.make_due(&id).await;

    // Worker B claims attempt 2.
    let claimed_b = db.store().claim_due(10, LEASE).await.expect("claim B");
    assert_eq!(claimed_b.len(), 1);
    assert_eq!(claimed_b[0].id, id);
    assert_eq!(claimed_b[0].attempts, 2);

    // Worker B finishes delivery and records Final(Delivered).
    db.store()
        .record(
            &claimed_b[0],
            RecordStep::Final(FinalOutcome::Delivered { http_status: 201 }),
            earn.as_ref(),
        )
        .await
        .expect("record B Final");

    assert_eq!(db.status(&id).await, "DELIVERED");
    assert_eq!(earn.finals().len(), 1);

    // Worker A's late response arrives and tries to record a Retry.
    db.store()
        .record(
            &claimed_a[0],
            RecordStep::Retry {
                after: Duration::from_secs(60),
                http_status: Some(503),
                error: "late timeout after lease lapsed".to_string(),
            },
            earn.as_ref(),
        )
        .await
        .expect("record A late Retry");

    // Status remains DELIVERED (not reopened to PENDING) and on_final is not called again.
    assert_eq!(db.status(&id).await, "DELIVERED");
    assert_eq!(earn.finals().len(), 1);
    db.finish().await;
}

#[tokio::test]
async fn late_final_after_lapsed_lease_leaves_final_status_unchanged_and_runs_on_final_once() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let id = db
        .enqueue(&EARN, "tx-2", &key(), json!({"transactionId": "tx-2"}))
        .await;

    // Worker A claims attempt 1.
    let claimed_a = db.store().claim_due(10, LEASE).await.expect("claim A");
    assert_eq!(claimed_a.len(), 1);
    assert_eq!(claimed_a[0].id, id);
    assert_eq!(claimed_a[0].attempts, 1);

    // Worker A's lease expires.
    db.make_due(&id).await;

    // Worker B claims attempt 2.
    let claimed_b = db.store().claim_due(10, LEASE).await.expect("claim B");
    assert_eq!(claimed_b.len(), 1);
    assert_eq!(claimed_b[0].id, id);
    assert_eq!(claimed_b[0].attempts, 2);

    // Worker B finishes delivery and records Final(Delivered).
    db.store()
        .record(
            &claimed_b[0],
            RecordStep::Final(FinalOutcome::Delivered { http_status: 201 }),
            earn.as_ref(),
        )
        .await
        .expect("record B Final");

    assert_eq!(db.status(&id).await, "DELIVERED");
    assert_eq!(earn.finals().len(), 1);

    // Worker A's late response arrives and tries to record Final(Refused).
    db.store()
        .record(
            &claimed_a[0],
            RecordStep::Final(FinalOutcome::Refused {
                http_status: 400,
                error: "late refused after lease lapsed".to_string(),
            }),
            earn.as_ref(),
        )
        .await
        .expect("record A late Final");

    // Status remains DELIVERED (not overwritten to FAILED) and on_final ran only once.
    assert_eq!(db.status(&id).await, "DELIVERED");
    assert_eq!(earn.finals().len(), 1);
    assert_eq!(
        earn.finals()[0],
        (id.clone(), FinalOutcome::Delivered { http_status: 201 })
    );
    db.finish().await;
}

#[tokio::test]
async fn late_retry_and_final_while_reclaimed_row_is_still_pending_do_not_move_lease_or_call_on_final()
 {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let id = db
        .enqueue(&EARN, "tx-3", &key(), json!({"transactionId": "tx-3"}))
        .await;

    // Worker A claims attempt 1.
    let claimed_a = db.store().claim_due(10, LEASE).await.expect("claim A");
    assert_eq!(claimed_a[0].attempts, 1);

    // Lease expires.
    db.make_due(&id).await;

    // Worker B claims attempt 2.
    let claimed_b = db.store().claim_due(10, LEASE).await.expect("claim B");
    assert_eq!(claimed_b[0].attempts, 2);

    let row_before = db.row(&id).await;
    assert_eq!(row_before.status, "PENDING");
    assert_eq!(row_before.attempts, 2);

    // Worker A sends late Retry (with attempts=1). Should match 0 rows.
    db.store()
        .record(
            &claimed_a[0],
            RecordStep::Retry {
                after: Duration::from_secs(3600),
                http_status: Some(500),
                error: "timeout".to_string(),
            },
            earn.as_ref(),
        )
        .await
        .expect("record A late Retry");

    let row_after_retry = db.row(&id).await;
    assert_eq!(row_after_retry.status, "PENDING");
    assert_eq!(row_after_retry.attempts, 2);
    // Worker B's lease was not overwritten with A's 3600s wait.
    assert!(row_after_retry.due_in_secs < 120.0);

    // Worker A sends late Final (with attempts=1). Should match 0 rows and roll back.
    db.store()
        .record(
            &claimed_a[0],
            RecordStep::Final(FinalOutcome::Refused {
                http_status: 500,
                error: "late refused".to_string(),
            }),
            earn.as_ref(),
        )
        .await
        .expect("record A late Final");

    assert_eq!(db.status(&id).await, "PENDING");
    assert_eq!(earn.finals().len(), 0);

    // Now Worker B finishes normally.
    db.store()
        .record(
            &claimed_b[0],
            RecordStep::Final(FinalOutcome::Delivered { http_status: 200 }),
            earn.as_ref(),
        )
        .await
        .expect("record B Final");

    assert_eq!(db.status(&id).await, "DELIVERED");
    assert_eq!(earn.finals().len(), 1);
    db.finish().await;
}
