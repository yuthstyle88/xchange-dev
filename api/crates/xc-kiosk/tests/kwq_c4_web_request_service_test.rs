//! KWQ-C4 — `WebRequestService` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D2,
//! D3, D8, D12; §4.4 B3, B4, B11a, B12): the member's own web request on the caller's
//! transaction — create (no ticket, one open request per member per branch), a fresh QR, and the
//! one-press cancel, before and after the staff scan.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;
mod kwq_common;

use std::sync::Arc;

use chrono::Duration;
use kiosk_service_common::*;
use kwq_common::RecordingQueue;
use pos_error::AppError;
use xc_kernel::queue_tickets::QueueTickets;
use xc_kiosk::application::kiosk::web_request::WebRequestService;

const MINUTE: i64 = 60_000;

struct World {
    db: TestDb,
    kiosk: KioskService,
    web: WebRequestService,
    queue: Arc<RecordingQueue>,
}

async fn world() -> World {
    let db = TestDb::start().await;
    let kiosk = db.service(FakePorts::standard());
    let queue = Arc::new(RecordingQueue::default());
    let web = WebRequestService::new(kiosk.clone(), queue.clone() as Arc<dyn QueueTickets>);
    World {
        db,
        kiosk,
        web,
        queue,
    }
}

impl World {
    async fn create(&self, b: &str, customer: &str, id: &str, token: &str, ms: i64) -> Result<String, AppError> {
        let mut tx = self.db.begin().await;
        let ip = format!("198.51.100.{}", id.len() + token.len());
        let out = self
            .web
            .create_in(&mut tx, &branch(b), customer, from_ip(buy100(id), &ip), token, at(ms))
            .await;
        match out {
            Ok(created) => {
                tx.commit().await.expect("commit");
                assert_eq!(created.request_id, id);
                assert_eq!(created.expires_at, exact(ms) + Duration::minutes(60));
                Ok(created.request_id)
            }
            Err(e) => {
                tx.rollback().await.expect("rollback");
                Err(e)
            }
        }
    }

    async fn status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
            .bind(id)
            .fetch_one(&self.db.pool)
            .await
            .expect("status")
    }
}

fn not_found<T: std::fmt::Debug>(r: &Result<T, AppError>) -> bool {
    matches!(r, Err(AppError::NotFound { .. }))
}

#[tokio::test(flavor = "multi_thread")]
async fn create_makes_a_quoted_request_with_no_ticket_and_one_per_member_per_branch() {
    let w = world().await;
    w.create(MAIN, "cust-1", "web-1", "tok-1", 0).await.expect("created");
    assert_eq!(w.status("web-1").await, "QUOTED");
    assert!(w.queue.issues().is_empty(), "no ticket, no number (B3)");
    assert_eq!(
        w.kiosk.find_by_token(&branch(MAIN), "tok-1", at(MINUTE)).await.expect("found"),
        "web-1"
    );

    let again = w.create(MAIN, "cust-1", "web-2", "tok-2", MINUTE).await;
    match again {
        Err(AppError::Conflict(msg)) => assert_eq!(msg, "alreadyQueued:web-1"),
        other => panic!("B4: expected Conflict(alreadyQueued:web-1), got {other:?}"),
    }
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kiosk_requests WHERE id = 'web-2'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count")
            == 0
    );
    // another branch, and another member, are not blocked
    w.create(B2, "cust-1", "web-3", "tok-3", MINUTE).await.expect("another branch");
    w.create(MAIN, "cust-2", "web-4", "tok-4", MINUTE).await.expect("another member");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_qr_replaces_the_token_only_for_the_members_own_open_request() {
    let w = world().await;
    let main = branch(MAIN);
    w.create(MAIN, "cust-1", "web-1", "tok-old", 0).await.expect("created");

    let mut tx = w.db.begin().await;
    let expires = w
        .web
        .reissue_qr_in(&mut tx, &main, "cust-1", "web-1", "tok-new", at(MINUTE))
        .await
        .expect("reissued");
    tx.commit().await.expect("commit");
    assert_eq!(expires, exact(0) + Duration::minutes(60), "a reissue keeps the expiry");
    assert!(not_found(&w.kiosk.find_by_token(&main, "tok-old", at(2 * MINUTE)).await));
    assert_eq!(
        w.kiosk.find_by_token(&main, "tok-new", at(2 * MINUTE)).await.expect("new token"),
        "web-1"
    );

    // another member's request, an unknown one, another branch: one answer, nothing changed
    for (b, customer, id) in [(MAIN, "cust-2", "web-1"), (MAIN, "cust-1", "nope"), (B2, "cust-1", "web-1")] {
        let mut tx = w.db.begin().await;
        let r = w
            .web
            .reissue_qr_in(&mut tx, &branch(b), customer, id, "tok-evil", at(2 * MINUTE))
            .await;
        tx.rollback().await.expect("rollback");
        assert!(not_found(&r), "{b} {customer} {id}: {r:?}");
    }
    assert_eq!(
        w.kiosk.find_by_token(&main, "tok-new", at(3 * MINUTE)).await.expect("still the new token"),
        "web-1"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_member_cancels_before_and_after_the_scan_in_one_press() {
    let w = world().await;
    let main = branch(MAIN);
    w.create(MAIN, "cust-1", "web-q", "tok-q", 0).await.expect("created");

    // before the scan: QUOTED → CANCELLED, no ticket to end
    let mut tx = w.db.begin().await;
    w.web
        .cancel_in(&mut tx, &main, "cust-1", "web-q", at(MINUTE))
        .await
        .expect("cancel a QUOTED request");
    tx.commit().await.expect("commit");
    assert_eq!(w.status("web-q").await, "CANCELLED");
    let reason: Option<String> =
        sqlx::query_scalar("SELECT cancel_reason FROM kiosk_requests WHERE id = 'web-q'")
            .fetch_one(&w.db.pool)
            .await
            .expect("reason");
    assert_eq!(reason.as_deref(), Some("CUSTOMER_CANCELLED"));
    let lines: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM test_audit WHERE kind = 'KIOSK_CANCELLED' AND subject = 'web-q'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(lines, 1, "one KIOSK_CANCELLED line");

    // after the cancel the member may make a new request (D12 counts only open ones)
    w.create(MAIN, "cust-1", "web-w", "tok-w", 2 * MINUTE).await.expect("a new request");
    w.kiosk
        .issue_by_token(&main, "tok-w", at(3 * MINUTE), w.queue.clone() as Arc<dyn QueueTickets>)
        .await
        .expect("scanned");
    assert_eq!(w.status("web-w").await, "WAITING");

    // another member cannot cancel it
    let mut tx = w.db.begin().await;
    let r = w.web.cancel_in(&mut tx, &main, "cust-2", "web-w", at(4 * MINUTE)).await;
    tx.rollback().await.expect("rollback");
    assert!(not_found(&r), "{r:?}");

    // after the scan: WAITING → CANCELLED and the ticket is ended through the queue
    let mut tx = w.db.begin().await;
    w.web
        .cancel_in(&mut tx, &main, "cust-1", "web-w", at(4 * MINUTE))
        .await
        .expect("cancel a WAITING request");
    tx.commit().await.expect("commit");
    assert_eq!(w.status("web-w").await, "CANCELLED");
    assert_eq!(w.queue.cancels(), vec![(MAIN.to_owned(), "web-w".to_owned())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_at_the_counter_cannot_be_cancelled_by_the_member() {
    let w = world().await;
    let main = branch(MAIN);
    w.create(MAIN, "cust-1", "web-a", "tok-a", 0).await.expect("created");
    sqlx::query("UPDATE kiosk_requests SET status = 'ACCEPTED', staff = 'front1' WHERE id = 'web-a'")
        .execute(&w.db.pool)
        .await
        .expect("at the counter");
    let mut tx = w.db.begin().await;
    let r = w.web.cancel_in(&mut tx, &main, "cust-1", "web-a", at(MINUTE)).await;
    tx.rollback().await.expect("rollback");
    assert!(matches!(r, Err(AppError::Domain(_))), "{r:?}");
    assert_eq!(w.status("web-a").await, "ACCEPTED");
}
