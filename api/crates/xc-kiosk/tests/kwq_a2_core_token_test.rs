//! KWQ-A2 — the request core's web half (design `docs/design/blueprint/kiosk-shared-phone.md`
//! §4.2 D1, D2, D5, D7; §4.4 B1, B3, B6–B8, B13): `quote_for` stores the member and the request
//! token's hash with the quote; `issue_by_token` issues exactly one ticket on the staff scan and
//! the same one on a repeat; `find_by_token` gives the desk the id; every token that does not
//! lead to this branch's live request gets one and the same refusal.
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
use xc_kernel::queue_tickets::{QueueTicketSource, QueueTickets};
use xc_kiosk::domain::kiosk::value_objects::customer_quote::CustomerQuote;

const MINUTE: i64 = 60_000;

fn member(token: &str) -> Option<CustomerQuote> {
    Some(CustomerQuote {
        customer_id: "cust-1".to_owned(),
        token: token.to_owned(),
    })
}

async fn column(db: &TestDb, sql: &str) -> Option<String> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&db.pool)
        .await
        .expect("read one value")
}

fn is_not_found<T: std::fmt::Debug>(result: Result<T, AppError>) -> bool {
    matches!(result, Err(AppError::NotFound { .. }))
}

/// B1/B3: `quote_for` without a member is `quote`; with one it stores the member and only the
/// token's SHA-256 (hex), and issues no ticket.
#[tokio::test(flavor = "multi_thread")]
async fn quote_for_stores_the_member_and_the_token_hash_and_issues_nothing() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);

    let kiosk = service
        .quote_for(&main, buy100("kiosk-1"), None, at(0))
        .await
        .expect("a kiosk quote");
    assert_eq!(kiosk.request.status, KioskStatus::Quoted);
    let web = service
        .quote_for(&main, from_ip(buy100("web-1"), "198.51.100.2"), member("tok-1"), at(1))
        .await
        .expect("a web quote");
    assert_eq!(web.request.status, KioskStatus::Quoted);

    assert_eq!(
        column(&db, "SELECT customer_id FROM kiosk_requests WHERE id = 'web-1'").await,
        Some("cust-1".to_owned())
    );
    assert_eq!(
        column(&db, "SELECT customer_id FROM kiosk_requests WHERE id = 'kiosk-1'").await,
        None
    );
    assert_eq!(
        column(&db, "SELECT cancel_token_hash FROM kiosk_requests WHERE id = 'web-1'").await,
        column(&db, "SELECT encode(sha256('tok-1'::bytea), 'hex')").await
    );
    assert_eq!(
        column(&db, "SELECT cancel_token_hash FROM kiosk_requests WHERE id = 'kiosk-1'").await,
        None
    );
    assert_eq!(db.count("queue_tickets").await, 0, "a web submit issues no ticket");
}

/// B6/B7: the scan confirms the QUOTED request, issues one kiosk ticket under the confirm's key,
/// links the member; a second scan answers the same ticket and issues no second number.
#[tokio::test(flavor = "multi_thread")]
async fn the_scan_issues_one_ticket_links_the_member_and_a_repeat_answers_the_same() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    let queue = Arc::new(RecordingQueue::default());
    service
        .quote_for(&main, buy100("web-1"), member("tok-1"), at(0))
        .await
        .expect("a web quote");

    let first = service
        .issue_by_token(&main, "tok-1", at(MINUTE), queue.clone() as Arc<dyn QueueTickets>)
        .await
        .expect("the scan issues");
    assert_eq!(first.request_id, "web-1");
    assert_eq!(first.ticket.display_number, "W001");
    assert_eq!(first.branch_name, service_branch_name(&service).await);
    assert!(!first.timezone.is_empty(), "the branch's timezone travels with the ticket");
    assert_eq!(
        column(&db, "SELECT status FROM kiosk_requests WHERE id = 'web-1'").await,
        Some("WAITING".to_owned())
    );
    assert_eq!(
        queue.issues().first().map(|(b, s, k)| (b.clone(), s.clone(), k.clone())),
        Some((
            MAIN.to_owned(),
            QueueTicketSource::Kiosk {
                request_id: "web-1".to_owned()
            },
            "kiosk-confirm:web-1".to_owned()
        ))
    );
    assert_eq!(
        queue.links(),
        vec![(MAIN.to_owned(), "web-1".to_owned(), "cust-1".to_owned())]
    );

    let again = service
        .issue_by_token(&main, "tok-1", at(2 * MINUTE), queue.clone() as Arc<dyn QueueTickets>)
        .await
        .expect("a repeat scan answers");
    assert_eq!(again, first, "the same ticket, the same time, the same branch");
    assert_eq!(queue.tickets().len(), 1, "never a second number");
    // the request token is still the request's key: the scan minted no cancel token over it
    assert_eq!(
        column(&db, "SELECT cancel_token_hash FROM kiosk_requests WHERE id = 'web-1'").await,
        column(&db, "SELECT encode(sha256('tok-1'::bytea), 'hex')").await
    );
}

async fn service_branch_name(service: &KioskService) -> String {
    service
        .public_rates(&branch(MAIN))
        .await
        .expect("MAIN's rates")
        .branch_name
}

/// B8/B13: another branch's token, an unknown one, an expired one, a reissued-away one and a
/// cancelled request all answer the same `NotFound`, from both the scan and the desk lookup,
/// and nothing is issued.
#[tokio::test(flavor = "multi_thread")]
async fn every_token_that_leads_nowhere_answers_the_same_not_found() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let (main, b2) = (branch(MAIN), branch(B2));
    let queue = Arc::new(RecordingQueue::default());
    let q = || queue.clone() as Arc<dyn QueueTickets>;

    service
        .quote_for(&main, buy100("web-main"), member("tok-main"), at(0))
        .await
        .expect("MAIN's web quote");
    service
        .quote_for(&main, from_ip(buy100("web-old"), "198.51.100.3"), member("tok-old"), at(0))
        .await
        .expect("a quote whose token is replaced");
    sqlx::query(
        "UPDATE kiosk_requests SET cancel_token_hash = encode(sha256('tok-new'::bytea), 'hex') \
         WHERE id = 'web-old'",
    )
    .execute(&db.pool)
    .await
    .expect("reissue the QR");

    // another branch
    assert!(is_not_found(service.issue_by_token(&b2, "tok-main", at(MINUTE), q()).await));
    assert!(is_not_found(service.find_by_token(&b2, "tok-main", at(MINUTE)).await));
    // unknown
    assert!(is_not_found(service.issue_by_token(&main, "nope", at(MINUTE), q()).await));
    assert!(is_not_found(service.find_by_token(&main, "nope", at(MINUTE)).await));
    // reissued-away
    assert!(is_not_found(service.issue_by_token(&main, "tok-old", at(MINUTE), q()).await));
    assert!(is_not_found(service.find_by_token(&main, "tok-old", at(MINUTE)).await));
    // past the purge window, though not purged yet
    assert!(is_not_found(service.issue_by_token(&main, "tok-main", at(61 * MINUTE), q()).await));
    assert!(is_not_found(service.find_by_token(&main, "tok-main", at(61 * MINUTE)).await));
    assert!(queue.issues().is_empty(), "nothing was issued");

    // a cancelled request
    sqlx::query(
        "UPDATE kiosk_requests SET status = 'CANCELLED', cancelled_at = now(), \
         cancelled_by = 'test', cancel_reason = 'CUSTOMER_LEFT' WHERE id = 'web-old'",
    )
    .execute(&db.pool)
    .await
    .expect("cancel it");
    assert!(is_not_found(service.issue_by_token(&main, "tok-new", at(MINUTE), q()).await));
    assert!(is_not_found(service.find_by_token(&main, "tok-new", at(MINUTE)).await));
    assert!(queue.issues().is_empty(), "nothing was issued");
}

/// B13: the desk's lookup answers the id of a live request, before and after the scan.
#[tokio::test(flavor = "multi_thread")]
async fn the_desk_lookup_answers_the_request_id_before_and_after_the_scan() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    let queue = Arc::new(RecordingQueue::default());
    service
        .quote_for(&main, buy100("web-1"), member("tok-1"), at(0))
        .await
        .expect("a web quote");

    assert_eq!(
        service.find_by_token(&main, "tok-1", at(MINUTE)).await.expect("found"),
        "web-1"
    );
    service
        .issue_by_token(&main, "tok-1", at(MINUTE), queue as Arc<dyn QueueTickets>)
        .await
        .expect("issued");
    assert_eq!(
        service.find_by_token(&main, "tok-1", at(2 * MINUTE)).await.expect("found"),
        "web-1"
    );
    assert_eq!(db.count("queue_tickets").await, 0, "the fake queue wrote no row");
}
