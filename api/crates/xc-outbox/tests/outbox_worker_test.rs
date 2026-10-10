//! OB-1 up-front tests — `OutboxWorker` and `PgOutboxStore` (design broker-seam.md §5.2, §11
//! "Up-front tests per ticket: OB-1"). Written before the implementation by the ticket author.
//! Do not edit, delete or weaken; add tests in a file of your own.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use serde_json::json;

fn handlers(list: &[&Arc<FakeHandler>]) -> Vec<Arc<dyn OutboxHandler>> {
    list.iter()
        .map(|h| (*h).clone() as Arc<dyn OutboxHandler>)
        .collect()
}

// ── outcomes ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_2xx_marks_the_row_delivered_and_calls_on_final_once() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;

    let report = worker.tick().await.expect("tick");
    assert_eq!(report.delivered, 1);

    let row = db.row(&id).await;
    assert_eq!(row.status, "DELIVERED");
    assert!(row.delivered, "delivered_at is set");
    assert_eq!(row.last_http_status, Some(201));
    assert_eq!(row.attempts, 1);
    assert_eq!(
        earn.finals(),
        vec![(id.clone(), FinalOutcome::Delivered { http_status: 201 })]
    );

    worker.tick().await.expect("second tick");
    assert_eq!(
        earn.deliveries_of(&id),
        1,
        "a delivered row is never sent again"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_final_refusal_marks_the_row_failed_and_calls_on_final() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    let refused = FinalOutcome::Refused {
        http_status: 404,
        error: "unknown code".to_owned(),
    };
    earn.answer(&id, vec![DeliveryResult::Final(refused.clone())]);

    let report = worker.tick().await.expect("tick");
    assert_eq!(report.failed, 1);
    let row = db.row(&id).await;
    assert_eq!(row.status, "FAILED");
    assert_eq!(row.last_http_status, Some(404));
    assert!(!row.delivered);
    assert_eq!(earn.finals(), vec![(id, refused)]);
    db.finish().await;
}

#[tokio::test]
async fn a_retry_stays_pending_and_waits_the_first_backoff() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    earn.answer(&id, vec![retry(503)]);

    let report = worker.tick().await.expect("tick");
    assert_eq!(report.retried, 1);
    let row = db.row(&id).await;
    assert_eq!(row.status, "PENDING");
    assert_eq!(row.attempts, 1);
    assert_eq!(row.last_http_status, Some(503));
    assert_eq!(row.last_error.as_deref(), Some("upstream answered 503"));
    assert!(
        (3.5..=6.0).contains(&row.due_in_secs),
        "after attempt 1 the default waits 5 s, got {} s",
        row.due_in_secs
    );
    assert!(earn.finals().is_empty(), "no final answer yet");

    worker.tick().await.expect("tick inside the backoff");
    assert_eq!(
        earn.deliveries_of(&id),
        1,
        "not sent again before the backoff ends"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_second_retry_waits_twice_as_long() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    earn.answer(&id, vec![retry(503), retry(503)]);

    worker.tick().await.expect("attempt 1");
    db.make_due(&id).await;
    worker.tick().await.expect("attempt 2");
    let row = db.row(&id).await;
    assert_eq!(row.attempts, 2);
    assert!(
        (8.5..=11.0).contains(&row.due_in_secs),
        "10 s after attempt 2, got {}",
        row.due_in_secs
    );
    db.finish().await;
}

#[tokio::test]
async fn a_handlers_own_retry_policy_sets_the_wait() {
    let db = TestDb::start().await;
    let policy = RetryPolicy {
        first: Duration::from_secs(60),
        cap: Duration::from_secs(3600),
        expire_after: Duration::from_secs(86_400),
    };
    let earn = FakeHandler::with_policy(&EARN, Some(policy));
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    earn.answer(&id, vec![retry(429)]);

    worker.tick().await.expect("tick");
    let due = db.row(&id).await.due_in_secs;
    assert!(
        (58.0..=61.0).contains(&due),
        "the handler's 60 s, got {due} s"
    );
    db.finish().await;
}

#[tokio::test]
async fn last_error_is_cut_to_500_characters() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    earn.answer(
        &id,
        vec![DeliveryResult::Retry {
            http_status: None,
            error: "ข".repeat(2_000),
        }],
    );

    worker.tick().await.expect("tick");
    let error = db.row(&id).await.last_error.expect("an error is stored");
    assert!(!error.is_empty());
    assert!(
        error.chars().count() <= 500,
        "got {} characters",
        error.chars().count()
    );
    db.finish().await;
}

#[tokio::test]
async fn a_failing_on_final_rolls_the_final_status_back() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    earn.fail_on_final(true);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;

    let _ = worker.tick().await;
    let row = db.row(&id).await;
    assert_eq!(
        row.status, "PENDING",
        "status and on_final share one transaction"
    );
    assert!(!row.delivered);

    earn.fail_on_final(false);
    db.make_due(&id).await;
    worker.tick().await.expect("tick");
    assert_eq!(db.status(&id).await, "DELIVERED");
    assert_eq!(earn.finals().len(), 1);
    db.finish().await;
}

#[tokio::test]
async fn the_handler_sees_the_rows_key_and_attempt_count() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let k = key();
    let id = db
        .enqueue(&EARN, "tx-1", &k, json!({"transactionId": "tx-1"}))
        .await;
    earn.answer(&id, vec![retry(503)]);

    worker.tick().await.expect("attempt 1");
    db.make_due(&id).await;
    worker.tick().await.expect("attempt 2");
    assert_eq!(earn.seen(), vec![(id.clone(), k.clone(), 1), (id, k, 2)]);
    db.finish().await;
}

// ── per-aggregate FIFO ──────────────────────────────────────────────────────

#[tokio::test]
async fn fifo_a_reversal_waits_while_the_earn_of_its_transaction_is_in_backoff() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let reverse = FakeHandler::new(&REVERSE);
    let worker = db.worker(handlers(&[&earn, &reverse]));

    let e = db
        .enqueue(&EARN, "tx-1", "earn:tx-1", json!({"transactionId": "tx-1"}))
        .await;
    earn.answer(&e, vec![retry(503)]);
    worker.tick().await.expect("the earn fails once");
    let r = db
        .enqueue(
            &REVERSE,
            "tx-1",
            "rev:tx-1",
            json!({"transactionId": "tx-1"}),
        )
        .await;

    worker.tick().await.expect("tick while the earn waits");
    worker.tick().await.expect("again");
    assert_eq!(
        reverse.deliveries_of(&r),
        0,
        "the reversal never overtakes the earn"
    );
    assert_eq!(db.status(&r).await, "PENDING");

    db.make_due(&e).await;
    drain(&worker, 5).await;
    assert_eq!(db.status(&e).await, "DELIVERED");
    assert_eq!(db.status(&r).await, "DELIVERED");
    db.finish().await;
}

#[tokio::test]
async fn fifo_an_earlier_row_that_fails_for_good_releases_the_next() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let reverse = FakeHandler::new(&REVERSE);
    let worker = db.worker(handlers(&[&earn, &reverse]));

    let e = db
        .enqueue(&EARN, "tx-1", "earn:tx-1", json!({"transactionId": "tx-1"}))
        .await;
    let r = db
        .enqueue(
            &REVERSE,
            "tx-1",
            "rev:tx-1",
            json!({"transactionId": "tx-1"}),
        )
        .await;
    earn.answer(
        &e,
        vec![DeliveryResult::Final(FinalOutcome::Refused {
            http_status: 422,
            error: "no".into(),
        })],
    );

    drain(&worker, 5).await;
    assert_eq!(db.status(&e).await, "FAILED");
    assert_eq!(
        db.status(&r).await,
        "DELIVERED",
        "a final failure does not hold the aggregate"
    );
    db.finish().await;
}

#[tokio::test]
async fn fifo_is_per_destination_and_per_aggregate() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let gl = FakeHandler::new(&GL);
    let worker = db.worker(handlers(&[&earn, &gl]));

    let e1 = db
        .enqueue(&EARN, "tx-1", "earn:tx-1", json!({"transactionId": "tx-1"}))
        .await;
    earn.answer(&e1, vec![retry(503)]);
    worker.tick().await.expect("tx-1's earn goes into backoff");

    let gl1 = db
        .enqueue(&GL, "tx-1", "gl:tx-1", json!({"transactionId": "tx-1"}))
        .await;
    let e2 = db
        .enqueue(&EARN, "tx-2", "earn:tx-2", json!({"transactionId": "tx-2"}))
        .await;
    worker.tick().await.expect("tick");

    assert_eq!(
        db.status(&gl1).await,
        "DELIVERED",
        "another destination of tx-1 is not held"
    );
    assert_eq!(
        db.status(&e2).await,
        "DELIVERED",
        "another transaction to loyalty is not held"
    );
    assert_eq!(db.status(&e1).await, "PENDING");
    db.finish().await;
}

#[tokio::test]
async fn fifo_delivers_one_aggregates_rows_in_enqueue_order() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let mut ids = Vec::new();
    for n in 0..5 {
        ids.push(
            db.enqueue(&EARN, "tx-1", &format!("k{n}"), json!({"n": n}))
                .await,
        );
    }
    drain(&worker, 10).await;
    let order: Vec<String> = earn.seen().into_iter().map(|(id, _, _)| id).collect();
    assert_eq!(order, ids);
    db.finish().await;
}

// ── claim, lease, crash ─────────────────────────────────────────────────────

#[tokio::test]
async fn two_workers_never_take_the_same_row() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let mut ids = Vec::new();
    for n in 0..200 {
        // One aggregate per row, so FIFO never holds a row back.
        ids.push(
            db.enqueue(&EARN, &format!("tx-{n}"), &key(), json!({"n": n}))
                .await,
        );
    }
    let workers: Vec<Arc<OutboxWorker>> = (0..4)
        .map(|_| Arc::new(db.worker(handlers(&[&earn]))))
        .collect();

    within(MUST_FINISH, "four workers drain 200 rows", async {
        let mut tasks = Vec::new();
        for w in &workers {
            let w = w.clone();
            tasks.push(tokio::spawn(async move { drain(&w, 50).await }));
        }
        for t in tasks {
            t.await.expect("worker task");
        }
    })
    .await;

    for id in &ids {
        assert_eq!(earn.deliveries_of(id), 1, "row {id} delivered exactly once");
        assert_eq!(db.status(id).await, "DELIVERED");
    }
    db.finish().await;
}

#[tokio::test]
async fn a_claimed_row_is_not_claimed_again_within_the_lease() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;

    // A worker that claims and then dies before delivering.
    let claimed = db.store().claim_due(20, LEASE).await.expect("claim");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempts, 1, "the claim counts the attempt");
    let row = db.row(&id).await;
    assert!(
        (55.0..=61.0).contains(&row.due_in_secs),
        "a 60 s lease, got {} s",
        row.due_in_secs
    );

    worker.tick().await.expect("tick inside the lease");
    assert_eq!(earn.deliveries_of(&id), 0, "a leased row is left alone");
    db.finish().await;
}

#[tokio::test]
async fn a_crash_after_deliver_redelivers_once_after_the_lease_with_the_same_key() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let k = key();
    let id = db
        .enqueue(&EARN, "tx-1", &k, json!({"transactionId": "tx-1"}))
        .await;

    // Delivered, then the process died before recording: the row is still leased.
    db.store().claim_due(20, LEASE).await.expect("claim");
    db.make_due(&id).await; // the lease has run out

    drain(&worker, 5).await;
    assert_eq!(
        earn.seen(),
        vec![(id.clone(), k, 2)],
        "sent once more, same key, attempt 2"
    );
    assert_eq!(db.status(&id).await, "DELIVERED");
    db.finish().await;
}

// ── expiry and missing handlers ─────────────────────────────────────────────

#[tokio::test]
async fn a_row_older_than_seven_days_expires_without_delivery_and_calls_on_final() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    db.age(&id, "7 days 1 minute").await;

    let report = worker.tick().await.expect("tick");
    assert_eq!(report.expired, 1);
    assert_eq!(db.status(&id).await, "EXPIRED");
    assert_eq!(earn.deliveries_of(&id), 0, "an expired row is not sent");
    assert_eq!(earn.finals(), vec![(id, FinalOutcome::Expired)]);
    db.finish().await;
}

#[tokio::test]
async fn a_row_just_under_seven_days_is_still_delivered() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let id = db
        .enqueue(&EARN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    db.age(&id, "6 days 23 hours").await;

    worker.tick().await.expect("tick");
    assert_eq!(db.status(&id).await, "DELIVERED");
    db.finish().await;
}

#[tokio::test]
async fn an_expired_earn_releases_the_reversal_behind_it() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let reverse = FakeHandler::new(&REVERSE);
    let worker = db.worker(handlers(&[&earn, &reverse]));
    let e = db
        .enqueue(&EARN, "tx-1", "earn:tx-1", json!({"transactionId": "tx-1"}))
        .await;
    let r = db
        .enqueue(
            &REVERSE,
            "tx-1",
            "rev:tx-1",
            json!({"transactionId": "tx-1"}),
        )
        .await;
    db.age(&e, "8 days").await;

    drain(&worker, 5).await;
    assert_eq!(db.status(&e).await, "EXPIRED");
    assert_eq!(db.status(&r).await, "DELIVERED");
    db.finish().await;
}

#[tokio::test]
async fn an_event_with_no_handler_stays_pending_and_the_tick_succeeds() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    let orphan = db
        .enqueue(&ORPHAN, "tx-1", &key(), json!({"transactionId": "tx-1"}))
        .await;
    let fine = db
        .enqueue(&EARN, "tx-2", &key(), json!({"transactionId": "tx-2"}))
        .await;

    let report = worker
        .tick()
        .await
        .expect("a missing handler is not an error");
    assert_eq!(report.unhandled, 1);
    assert_eq!(db.status(&orphan).await, "PENDING");
    assert_eq!(
        db.status(&fine).await,
        "DELIVERED",
        "other rows are not held up"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_tick_with_nothing_due_reports_nothing() {
    let db = TestDb::start().await;
    let earn = FakeHandler::new(&EARN);
    let worker = db.worker(handlers(&[&earn]));
    assert_eq!(worker.tick().await.expect("tick"), TickReport::default());
    db.finish().await;
}
