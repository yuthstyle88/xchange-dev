//! KWQ-C3 — the request core for the web source (design `docs/design/blueprint/kiosk-shared-phone.md`
//! §4.2 D1, D4; §4.4 B1, B5): `quote_for_in` runs `quote_for`'s statements on the caller's
//! transaction (the v3 command seam's), and the core purges and scans with each source's window —
//! a web request (one with a member) waits 60 minutes, a kiosk quote 10.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;
mod kwq_common;

use std::sync::Arc;

use kiosk_service_common::*;
use kwq_common::RecordingQueue;
use pos_error::AppError;
use xc_kernel::queue_tickets::QueueTickets;
use xc_kiosk::domain::kiosk::value_objects::customer_quote::CustomerQuote;

const MINUTE: i64 = 60_000;

fn member(token: &str) -> Option<CustomerQuote> {
    Some(CustomerQuote {
        customer_id: "cust-1".to_owned(),
        token: token.to_owned(),
    })
}

async fn ids(db: &TestDb) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM kiosk_requests ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .expect("ids")
}

#[tokio::test(flavor = "multi_thread")]
async fn quote_for_in_writes_on_the_callers_transaction() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);

    // rolled back by the caller: nothing stays
    let mut tx = db.begin().await;
    let rolled = service
        .quote_for_in(&mut tx, &main, buy100("web-0"), member("tok-0"), at(0))
        .await
        .expect("quoted inside the caller's transaction");
    assert_eq!(rolled.request.status, KioskStatus::Quoted);
    tx.rollback().await.expect("rollback");
    assert!(ids(&db).await.is_empty(), "the caller's rollback takes the request with it");

    // committed by the caller: the request, its member and its token hash
    let mut tx = db.begin().await;
    service
        .quote_for_in(&mut tx, &main, buy100("web-1"), member("tok-1"), at(0))
        .await
        .expect("quoted");
    tx.commit().await.expect("commit");
    let row: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT customer_id, (cancel_token_hash = encode(sha256('tok-1'::bytea), 'hex'))::text \
         FROM kiosk_requests WHERE id = 'web-1'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("row");
    assert_eq!(row, (Some("cust-1".to_owned()), Some("true".to_owned())));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_next_quote_purges_a_kiosk_quote_after_ten_minutes_but_a_web_request_after_sixty() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    service
        .quote_for(&main, from_ip(buy100("kiosk-1"), "198.51.100.1"), None, at(0))
        .await
        .expect("kiosk quote");
    service
        .quote_for(&main, from_ip(buy100("web-1"), "198.51.100.2"), member("tok-1"), at(0))
        .await
        .expect("web request");

    service
        .quote(&main, from_ip(buy100("later-1"), "198.51.100.3"), at(11 * MINUTE))
        .await
        .expect("a quote 11 min later");
    assert_eq!(ids(&db).await, vec!["later-1", "web-1"], "the kiosk quote went, the web request stays");

    service
        .quote(&main, from_ip(buy100("later-2"), "198.51.100.4"), at(61 * MINUTE))
        .await
        .expect("a quote 61 min later");
    assert!(
        !ids(&db).await.contains(&"web-1".to_owned()),
        "after 60 minutes the web request is purged too"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scan_finds_a_web_request_for_sixty_minutes() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    let queue = Arc::new(RecordingQueue::default());
    service
        .quote_for(&main, buy100("web-1"), member("tok-1"), at(0))
        .await
        .expect("web request");
    service
        .quote_for(&main, from_ip(buy100("web-2"), "198.51.100.9"), member("tok-2"), at(0))
        .await
        .expect("another web request");

    let issued = service
        .issue_by_token(&main, "tok-1", at(59 * MINUTE), queue.clone() as Arc<dyn QueueTickets>)
        .await
        .expect("59 minutes after the quote the scan still issues");
    assert_eq!(issued.request_id, "web-1");
    assert_eq!(
        service.find_by_token(&main, "tok-2", at(59 * MINUTE)).await.expect("found"),
        "web-2"
    );
    let late = service.find_by_token(&main, "tok-2", at(61 * MINUTE)).await;
    assert!(matches!(late, Err(AppError::NotFound { .. })), "{late:?}");
}
