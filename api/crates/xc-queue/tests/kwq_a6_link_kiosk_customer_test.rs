//! KWQ-A6 — `QueueTickets::link_kiosk_customer_in` on `TicketIssuer` against a real PostgreSQL
//! (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D5, §4.4 B6): the staff scan of a
//! web request sets the member on the request's active ticket, so the member's account shows it
//! (`my_queue_ticket`). It never overwrites another member and never touches another ticket.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Contract (`[required]`): `Ok(true)` when the request's active (WAITING, CALLED or SERVING)
//! ticket now carries `customer_id`, including a repeat with the same member; `Ok(false)` when
//! there is no active ticket, or it already carries a different member (left unchanged).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use queue_common::*;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::{QueueTicketSource, QueueTickets};

fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, h, m, 0).unwrap()
}

/// Issues the kiosk ticket of `request` through the port; returns its id.
async fn issue(db: &TestDb, port: Arc<dyn QueueTickets>, request: &str) -> String {
    let request = request.to_owned();
    db.branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                let source = QueueTicketSource::Kiosk {
                    request_id: request.clone(),
                };
                port.issue_in(
                    tx,
                    &BranchId::new("MAIN"),
                    &source,
                    &format!("kiosk-confirm:{request}"),
                    at(3, 0),
                )
                .await
            })
        })
        .await
        .expect("issue")
        .id
}

async fn link(db: &TestDb, port: Arc<dyn QueueTickets>, request: &str, customer: &str) -> bool {
    let (request, customer) = (request.to_owned(), customer.to_owned());
    db.branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                port.link_kiosk_customer_in(tx, &BranchId::new("MAIN"), &request, &customer)
                    .await
            })
        })
        .await
        .expect("link")
}

async fn customer_of(db: &TestDb, ticket: &str) -> Option<String> {
    db.text(&format!(
        "SELECT customer_id FROM queue_tickets WHERE id = '{ticket}'::uuid"
    ))
    .await
}

#[tokio::test]
async fn the_member_is_set_on_the_requests_ticket_and_a_repeat_is_harmless() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    db.add_kiosk_request("MAIN", "KR-2").await;
    let port: Arc<dyn QueueTickets> = Arc::new(db.issuer());
    let t1 = issue(&db, port.clone(), "KR-1").await;
    let t2 = issue(&db, port.clone(), "KR-2").await;

    assert!(link(&db, port.clone(), "KR-1", "cust-1").await);
    assert_eq!(customer_of(&db, &t1).await.as_deref(), Some("cust-1"));
    assert_eq!(customer_of(&db, &t2).await, None, "another request's ticket is untouched");

    assert!(link(&db, port.clone(), "KR-1", "cust-1").await, "a repeat answers true");
    assert_eq!(customer_of(&db, &t1).await.as_deref(), Some("cust-1"));
    db.finish().await;
}

#[tokio::test]
async fn another_member_is_never_written_over_and_no_ticket_answers_false() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    let port: Arc<dyn QueueTickets> = Arc::new(db.issuer());
    let t1 = issue(&db, port.clone(), "KR-1").await;

    assert!(link(&db, port.clone(), "KR-1", "cust-1").await);
    assert!(!link(&db, port.clone(), "KR-1", "cust-2").await);
    assert_eq!(customer_of(&db, &t1).await.as_deref(), Some("cust-1"));

    assert!(!link(&db, port.clone(), "NO-SUCH", "cust-1").await, "no ticket");
    assert_eq!(
        db.count("SELECT count(*) FROM queue_tickets WHERE customer_id IS NOT NULL")
            .await,
        1
    );
    db.finish().await;
}

#[tokio::test]
async fn an_ended_ticket_is_not_linked() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    let port: Arc<dyn QueueTickets> = Arc::new(db.issuer());
    let t1 = issue(&db, port.clone(), "KR-1").await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE queue_tickets SET status = 'CANCELLED', completed_at = now(), \
         ended_reason = 'test' WHERE id = '{t1}'::uuid"
    )))
    .execute(&db.pool)
    .await
    .expect("end the ticket");

    assert!(!link(&db, port.clone(), "KR-1", "cust-1").await);
    assert_eq!(customer_of(&db, &t1).await, None);
    db.finish().await;
}
