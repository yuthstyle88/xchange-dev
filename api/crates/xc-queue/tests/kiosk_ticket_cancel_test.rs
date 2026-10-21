//! KC-Q — `QueueTickets::cancel_active_for_kiosk_in` and its `TicketIssuer` implementation
//! against a real PostgreSQL: one guarded `UPDATE` per outcome, one `SKIP`/`CANCEL` event, the
//! caller's transaction, and nothing written for a ticket that has already ended.
//! Written before the implementation by the ticket author (claude/studio-claude-10). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/kiosk-cancel.md` §3.2 (the table of guarded `UPDATE`s).
//!
//! Signatures this test assumes (`[required]` in the ticket):
//! - `xc_kernel::queue_tickets::KioskTicketOutcome { NoShow, Cancelled { reason: String } }`;
//! - `QueueTickets::cancel_active_for_kiosk_in(tx, branch, request_id, &outcome, now)
//!   -> BoxFuture<AppResult<bool>>`, implemented by `TicketIssuer`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use queue_common::*;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::{KioskTicketOutcome, QueueTicketSource, QueueTickets};

fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

/// 2026-11-02 03:00 UTC = 10:00 in Bangkok.
fn at(m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, 3, m, 0).unwrap()
}

fn cancelled(code: &str) -> KioskTicketOutcome {
    KioskTicketOutcome::Cancelled {
        reason: code.to_owned(),
    }
}

/// An open queue, kiosk request `request` (WAITING) and its walk-in ticket; returns the ticket id.
async fn kiosk_ticket(db: &TestDb, request: &str) -> String {
    db.add_kiosk_request("MAIN", request).await;
    let port: Arc<dyn QueueTickets> = Arc::new(db.issuer());
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
                    at(0),
                )
                .await
            })
        })
        .await
        .expect("issue the kiosk ticket")
        .id
}

/// Puts the ticket in `status` as the queue's own moves would leave it.
async fn set_status(db: &TestDb, ticket: &str, status: &str, teller: Option<&str>) {
    let teller = teller.map_or("NULL".to_owned(), |t| format!("'{t}'::uuid"));
    db.exec(&format!(
        "UPDATE queue_tickets SET status = '{status}', counter_teller_id = {teller}, \
             called_at = CASE WHEN '{status}' = 'WAITING' THEN NULL ELSE now() END, \
             served_at = CASE WHEN '{status}' IN ('SERVING', 'COMPLETED') THEN now() ELSE NULL END, \
             completed_at = CASE WHEN '{status}' = 'COMPLETED' THEN now() ELSE NULL END \
         WHERE id = '{ticket}'::uuid"
    ))
    .await;
}

/// Runs the port in its own transaction, as the issuer `port`.
async fn cancel(
    db: &TestDb,
    port: Arc<dyn QueueTickets>,
    request: &str,
    outcome: KioskTicketOutcome,
) -> Result<bool, AppError> {
    let request = request.to_owned();
    db.branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                port.cancel_active_for_kiosk_in(
                    tx,
                    &BranchId::new("MAIN"),
                    &request,
                    &outcome,
                    at(5),
                )
                .await
            })
        })
        .await
}

async fn ticket_row(db: &TestDb, ticket: &str) -> String {
    db.text(&format!(
        "SELECT status || ' ' || coalesce(ended_reason, '-') FROM queue_tickets WHERE id = '{ticket}'::uuid"
    ))
    .await
    .expect("the ticket row")
}

/// Every event of the ticket after its ISSUE: `<kind> <status> <reason>`, oldest first.
async fn later_events(db: &TestDb, ticket: &str) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT kind || ' ' || status || ' ' || coalesce(reason, '-') FROM queue_events \
         WHERE ticket_id = '{ticket}'::uuid AND kind <> 'ISSUE' ORDER BY at, created_at"
    )))
    .fetch_all(&db.pool)
    .await
    .expect("read the events");
    rows.into_iter().map(|(r,)| r).collect()
}

#[tokio::test]
async fn a_waiting_ticket_of_a_customer_who_left_becomes_no_show_with_one_skip_event() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let ticket = kiosk_ticket(&db, "KR-1").await;

    let moved = cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-1",
        KioskTicketOutcome::NoShow,
    )
    .await;

    assert!(moved.expect("the port answers"));
    assert_eq!(
        ticket_row(&db, &ticket).await,
        "NO_SHOW kioskCancelled:CUSTOMER_LEFT"
    );
    assert_eq!(
        later_events(&db, &ticket).await,
        ["SKIP NO_SHOW kioskCancelled:CUSTOMER_LEFT"],
        "NO_SHOW is recorded as SKIP (the event kinds of 20261101100000_queue.sql:37)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_called_ticket_of_a_customer_who_left_becomes_no_show() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let ticket = kiosk_ticket(&db, "KR-1").await;
    set_status(&db, &ticket, "CALLED", None).await;

    let moved = cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-1",
        KioskTicketOutcome::NoShow,
    )
    .await;

    assert!(moved.expect("the port answers"));
    assert_eq!(
        ticket_row(&db, &ticket).await,
        "NO_SHOW kioskCancelled:CUSTOMER_LEFT"
    );
    assert_eq!(
        later_events(&db, &ticket).await,
        ["SKIP NO_SHOW kioskCancelled:CUSTOMER_LEFT"]
    );
    db.finish().await;
}

#[tokio::test]
async fn a_waiting_or_called_ticket_cancelled_for_a_reason_becomes_cancelled_with_one_cancel_event()
{
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let waiting = kiosk_ticket(&db, "KR-1").await;
    let called = kiosk_ticket(&db, "KR-2").await;
    set_status(&db, &called, "CALLED", None).await;

    let first = cancel(&db, Arc::new(db.issuer()), "KR-1", cancelled("DUPLICATE")).await;
    let second = cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-2",
        cancelled("CUSTOMER_CANCELLED"),
    )
    .await;

    assert!(first.expect("the port answers"));
    assert!(second.expect("the port answers"));
    assert_eq!(
        ticket_row(&db, &waiting).await,
        "CANCELLED kioskCancelled:DUPLICATE"
    );
    assert_eq!(
        ticket_row(&db, &called).await,
        "CANCELLED kioskCancelled:CUSTOMER_CANCELLED"
    );
    assert_eq!(
        later_events(&db, &waiting).await,
        ["CANCEL CANCELLED kioskCancelled:DUPLICATE"]
    );
    assert_eq!(
        later_events(&db, &called).await,
        ["CANCEL CANCELLED kioskCancelled:CUSTOMER_CANCELLED"]
    );
    db.finish().await;
}

#[tokio::test]
async fn a_serving_ticket_is_cancelled_never_no_show_whatever_the_outcome() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let teller = db.add_user("teller-1").await;
    let left = kiosk_ticket(&db, "KR-1").await;
    let other = kiosk_ticket(&db, "KR-2").await;
    set_status(&db, &left, "SERVING", Some(&teller)).await;
    set_status(&db, &other, "SERVING", Some(&teller)).await;

    let first = cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-1",
        KioskTicketOutcome::NoShow,
    )
    .await;
    let second = cancel(&db, Arc::new(db.issuer()), "KR-2", cancelled("OTHER")).await;

    assert!(first.expect("the port answers"));
    assert!(second.expect("the port answers"));
    assert_eq!(
        ticket_row(&db, &left).await,
        "CANCELLED kioskCancelled:CUSTOMER_LEFT",
        "NO_SHOW means never came to the counter, which is false once served"
    );
    assert_eq!(
        ticket_row(&db, &other).await,
        "CANCELLED kioskCancelled:OTHER"
    );
    assert_eq!(
        later_events(&db, &left).await,
        ["CANCEL CANCELLED kioskCancelled:CUSTOMER_LEFT"]
    );
    assert_eq!(
        later_events(&db, &other).await,
        ["CANCEL CANCELLED kioskCancelled:OTHER"]
    );
    db.finish().await;
}

#[tokio::test]
async fn an_ended_ticket_or_no_ticket_is_left_alone_and_answers_false() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let teller = db.add_user("teller-1").await;
    let mut ended = Vec::new();
    for (request, status) in [
        ("KR-C", "COMPLETED"),
        ("KR-X", "CANCELLED"),
        ("KR-N", "NO_SHOW"),
    ] {
        let ticket = kiosk_ticket(&db, request).await;
        set_status(&db, &ticket, status, Some(&teller)).await;
        db.exec(&format!(
            "UPDATE queue_tickets SET ended_reason = 'before' WHERE id = '{ticket}'::uuid"
        ))
        .await;
        ended.push((request, status, ticket));
    }
    db.add_kiosk_request("MAIN", "KR-NONE").await;

    for (request, status, ticket) in &ended {
        for outcome in [KioskTicketOutcome::NoShow, cancelled("DUPLICATE")] {
            let moved = cancel(&db, Arc::new(db.issuer()), request, outcome.clone()).await;
            assert!(!moved.expect("the port answers"), "{status} {outcome:?}");
        }
        assert_eq!(ticket_row(&db, ticket).await, format!("{status} before"));
        assert_eq!(
            later_events(&db, ticket).await,
            Vec::<String>::new(),
            "{status}"
        );
    }
    let none = cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-NONE",
        KioskTicketOutcome::NoShow,
    )
    .await;
    assert!(!none.expect("the port answers"), "a request with no ticket");
    let unknown = cancel(&db, Arc::new(db.issuer()), "KR-UNKNOWN", cancelled("OTHER")).await;
    assert!(!unknown.expect("the port answers"), "an id with no request");
    db.finish().await;
}

#[tokio::test]
async fn only_the_named_requests_ticket_moves() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let named = kiosk_ticket(&db, "KR-1").await;
    let bystander = kiosk_ticket(&db, "KR-2").await;

    cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-1",
        KioskTicketOutcome::NoShow,
    )
    .await
    .expect("the port answers");

    assert_eq!(
        ticket_row(&db, &named).await,
        "NO_SHOW kioskCancelled:CUSTOMER_LEFT"
    );
    assert_eq!(ticket_row(&db, &bystander).await, "WAITING -");
    assert_eq!(later_events(&db, &bystander).await, Vec::<String>::new());
    db.finish().await;
}

#[tokio::test]
async fn the_event_names_the_issuers_actor() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let manager = db.add_user("manager-1").await;
    let ticket = kiosk_ticket(&db, "KR-1").await;
    let anonymous = kiosk_ticket(&db, "KR-2").await;
    let acting = db
        .issuer()
        .acting_as(Actor::new(&manager, ORG_1, Vec::new(), None));

    cancel(&db, Arc::new(acting), "KR-1", cancelled("DUPLICATE"))
        .await
        .expect("the port answers");
    cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-2",
        KioskTicketOutcome::NoShow,
    )
    .await
    .expect("the port answers");

    assert_eq!(
        db.text(&format!(
            "SELECT actor_id::text FROM queue_events WHERE ticket_id = '{ticket}'::uuid AND kind = 'CANCEL'"
        ))
        .await,
        Some(manager)
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM queue_events WHERE ticket_id = '{anonymous}'::uuid \
             AND kind = 'SKIP' AND actor_id IS NULL"
        ))
        .await,
        1,
        "an issuer acting for nobody (the customer's own cancel) writes no actor"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_cancel_rolls_back_with_the_callers_transaction() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let ticket = kiosk_ticket(&db, "KR-1").await;
    let port: Arc<dyn QueueTickets> = Arc::new(db.issuer());

    let rolled_back = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                let moved = port
                    .cancel_active_for_kiosk_in(
                        tx,
                        &BranchId::new("MAIN"),
                        "KR-1",
                        &KioskTicketOutcome::NoShow,
                        at(5),
                    )
                    .await?;
                assert!(moved, "the ticket moves inside the transaction");
                Err::<(), _>(AppError::Validation(
                    "the caller fails after cancelling".to_owned(),
                ))
            })
        })
        .await;

    assert!(rolled_back.is_err());
    assert_eq!(ticket_row(&db, &ticket).await, "WAITING -");
    assert_eq!(later_events(&db, &ticket).await, Vec::<String>::new());
    db.finish().await;
}

#[tokio::test]
async fn the_branch_status_is_refreshed_in_the_same_transaction() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    kiosk_ticket(&db, "KR-1").await;
    let before = db
        .count("SELECT queue_version FROM branch_service_status WHERE branch_id = 'MAIN'")
        .await;

    cancel(
        &db,
        Arc::new(db.issuer()),
        "KR-1",
        KioskTicketOutcome::NoShow,
    )
    .await
    .expect("the port answers");

    let after = db
        .count("SELECT queue_version FROM branch_service_status WHERE branch_id = 'MAIN'")
        .await;
    assert!(
        after > before,
        "status.refresh_in runs after the move: queue_version {before} -> {after}"
    );
    db.finish().await;
}
