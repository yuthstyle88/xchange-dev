//! KWQ-A4 — `RequestIssueService` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D5,
//! D7, D9; §4.3a W8; §4.4 B6–B8, B10, B13): the staff scan answers
//! `{displayNumber, ticketId, ticketEscpos}`, the bytes being `TicketPrintJob` of that ticket in
//! the branch's timezone with the request QR `<origin>/q#<token>`; a repeat answers the same
//! bytes; the core's refusal passes through.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;
mod kwq_common;

use std::sync::Arc;

use chrono_tz::Tz;
use kiosk_service_common::*;
use kwq_common::RecordingQueue;
use pos_error::AppError;
use xc_kernel::queue_tickets::QueueTickets;
use xc_kiosk::application::kiosk::request_issue::RequestIssueService;
use xc_kiosk::domain::kiosk::value_objects::customer_quote::CustomerQuote;
use xc_kiosk::domain::kiosk::value_objects::ticket_print_job::TicketPrintJob;

const ORIGIN: &str = "https://xc-v3.example";

async fn setup() -> (TestDb, RequestIssueService, Arc<RecordingQueue>) {
    let db = TestDb::start().await;
    sqlx::query("UPDATE branches SET timezone = 'Asia/Bangkok' WHERE id = 'MAIN'")
        .execute(&db.pool)
        .await
        .expect("MAIN in Bangkok");
    let kiosk = db.service(FakePorts::standard());
    kiosk
        .quote_for(
            &branch(MAIN),
            buy100("web-1"),
            Some(CustomerQuote {
                customer_id: "cust-1".to_owned(),
                token: "tok-1".to_owned(),
            }),
            at(0),
        )
        .await
        .expect("a web quote");
    let queue = Arc::new(RecordingQueue::default());
    let issue = RequestIssueService::new(kiosk, queue.clone() as Arc<dyn QueueTickets>);
    (db, issue, queue)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scan_answers_the_number_the_ticket_id_and_the_printed_bytes() {
    let (_db, issue, queue) = setup().await;
    let issued = issue
        .issue_by_token(&branch(MAIN), "tok-1", ORIGIN, at(60_000))
        .await
        .expect("issued");
    let ticket = queue.tickets().pop().expect("one ticket");
    assert_eq!(issued.display_number, ticket.display_number);
    assert_eq!(issued.ticket_id, ticket.id);

    let bangkok: Tz = "Asia/Bangkok".parse().expect("tz");
    let branch_name = FakePorts::standard()
        .branch_name(&branch(MAIN))
        .await
        .expect("read")
        .expect("MAIN");
    let expected = TicketPrintJob {
        display_number: ticket.display_number.clone(),
        branch_name,
        issued_at: ticket.issued_at.with_timezone(&bangkok).fixed_offset(),
        qr_url: format!("{ORIGIN}/q#tok-1"),
    }
    .render();
    assert_eq!(issued.ticket_escpos, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeat_scan_answers_the_same_bytes() {
    let (_db, issue, queue) = setup().await;
    let first = issue
        .issue_by_token(&branch(MAIN), "tok-1", ORIGIN, at(60_000))
        .await
        .expect("issued");
    let again = issue
        .issue_by_token(&branch(MAIN), "tok-1", ORIGIN, at(300_000))
        .await
        .expect("issued again");
    assert_eq!(again, first);
    assert_eq!(queue.tickets().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_that_leads_nowhere_is_not_found_and_the_desk_lookup_answers_the_id() {
    let (_db, issue, queue) = setup().await;
    let refused = issue
        .issue_by_token(&branch(B2), "tok-1", ORIGIN, at(60_000))
        .await;
    assert!(
        matches!(refused, Err(AppError::NotFound { .. })),
        "{refused:?}"
    );
    assert!(queue.issues().is_empty());

    assert_eq!(
        issue
            .find_by_token(&branch(MAIN), "tok-1", at(60_000))
            .await
            .expect("found"),
        "web-1"
    );
    let unknown = issue.find_by_token(&branch(MAIN), "nope", at(60_000)).await;
    assert!(
        matches!(unknown, Err(AppError::NotFound { .. })),
        "{unknown:?}"
    );
}
