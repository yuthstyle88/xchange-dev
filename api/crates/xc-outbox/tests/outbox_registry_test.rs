//! OB-1 up-front tests — `OutboxRegistry` (design broker-seam.md §5.2). Written before the
//! implementation by the ticket author. Do not edit, delete or weaken; add tests in a file of
//! your own.

mod common;

use std::sync::Arc;

use common::*;
use pos_error::AppError;
use serde_json::json;

/// Same `event_type` as `EARN`, another destination.
static EARN_ELSEWHERE: EventKind = EventKind {
    event_type: "test.loyalty.earn",
    destination: "somewhere-else",
    aggregate_type: "transaction",
};

#[test]
fn two_handlers_with_one_event_type_fail_the_boot() {
    let a: Arc<dyn OutboxHandler> = FakeHandler::new(&EARN);
    let b: Arc<dyn OutboxHandler> = FakeHandler::new(&EARN_ELSEWHERE);
    match OutboxRegistry::new(vec![a, b]) {
        Err(AppError::Internal(message)) => {
            assert!(
                message.contains("test.loyalty.earn"),
                "names the event type: {message}"
            )
        }
        Err(other) => panic!("expected Internal, got {other:?}"),
        Ok(_) => panic!("a duplicate event type must fail the boot"),
    }
}

#[test]
fn a_handler_is_found_by_its_event_type_only() {
    let earn: Arc<dyn OutboxHandler> = FakeHandler::new(&EARN);
    let gl: Arc<dyn OutboxHandler> = FakeHandler::new(&GL);
    let registry = OutboxRegistry::new(vec![earn, gl]).expect("no duplicates");
    assert_eq!(
        registry.handler("test.loyalty.earn").expect("earn").kind(),
        &EARN
    );
    assert_eq!(registry.handler("test.gl").expect("gl").kind(), &GL);
    assert!(registry.handler("test.orphan").is_none());
    assert!(
        registry.handler("loyalty").is_none(),
        "a destination is not an event type"
    );
}

#[tokio::test]
async fn verify_refuses_a_pending_row_written_with_another_kind() {
    let db = TestDb::start().await;
    // Rows written by an older binary whose EARN went to "loyalty".
    db.enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    let moved: Arc<dyn OutboxHandler> = FakeHandler::new(&EARN_ELSEWHERE);
    let registry = OutboxRegistry::new(vec![moved]).expect("one handler");
    let out = registry.verify_against(&db.pool).await;
    assert!(matches!(out, Err(AppError::Internal(_))), "got {out:?}");
    db.finish().await;
}

#[tokio::test]
async fn verify_passes_matching_rows_and_rows_with_no_handler() {
    let db = TestDb::start().await;
    db.enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    db.enqueue(&ORPHAN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    let earn: Arc<dyn OutboxHandler> = FakeHandler::new(&EARN);
    let registry = OutboxRegistry::new(vec![earn]).expect("one handler");
    registry
        .verify_against(&db.pool)
        .await
        .expect("the kinds match");
    db.finish().await;
}
