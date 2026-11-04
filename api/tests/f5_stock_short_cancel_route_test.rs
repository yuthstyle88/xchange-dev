//! Ticket #6094 (F5): the desk cancels a WAITING kiosk request with the staff code `STOCK_SHORT`
//! through `POST /api/kiosk` `cancel`, and the reason is stored on the request and on its ticket
//! (`docs/design/blueprint/kiosk-cancel.md` §2, §3.2, as amended for #6094).
//!
//! Written before the implementation by the ticket author (claude/01TxaRJG). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Through the server's whole route table with real sessions (`kiosk_http_common::Counter`, which
//! opens `MAIN`'s queue, so `confirm` issues the request's walk-in ticket).
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{Counter, person};
use serde_json::json;

const ID: &str = "5c0c5f05-0000-4000-8000-0000000f5001";

/// `cancel_reason | cancel_note` of the request, `-` for a NULL note.
async fn request_reason(counter: &Counter, id: &str) -> String {
    let (reason, note): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT cancel_reason, cancel_note FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
    )
    .bind(rpc_common::MAIN)
    .bind(id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .expect("the request row");
    format!(
        "{} | {}",
        reason.as_deref().unwrap_or("-"),
        note.as_deref().unwrap_or("-")
    )
}

/// `status ended_reason` of the request's ticket.
async fn ticket_end(counter: &Counter, id: &str) -> String {
    let (status, ended): (String, Option<String>) = sqlx::query_as(
        "SELECT status, ended_reason FROM queue_tickets WHERE branch_id = $1 AND kiosk_request_id = $2",
    )
    .bind(rpc_common::MAIN)
    .bind(id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .expect("the request's ticket");
    format!("{status} {}", ended.as_deref().unwrap_or("-"))
}

#[actix_web::test]
async fn f5_the_desk_cancels_with_stock_short_and_it_is_recorded_on_request_and_ticket() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    let id = counter.waiting_request(ID).await;
    let answer = counter
        .ask(
            Some("FR1"),
            "cancel",
            json!({ "id": id, "branchId": rpc_common::MAIN, "reason": "STOCK_SHORT" }),
        )
        .await;
    assert_eq!(answer.status, 200, "cancel: {}", answer.text);
    assert_eq!(answer.json()["status"], "CANCELLED", "{}", answer.text);
    assert_eq!(request_reason(&counter, &id).await, "STOCK_SHORT | -");
    assert_eq!(
        ticket_end(&counter, &id).await,
        "CANCELLED kioskCancelled:STOCK_SHORT",
        "a stock cancel is not a no-show: only CUSTOMER_LEFT ends the ticket NO_SHOW (§2)"
    );
    counter.finish().await;
}

#[actix_web::test]
async fn f5_stock_short_keeps_the_tellers_note() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    let id = counter.waiting_request(ID).await;
    let answer = counter
        .ask(
            Some("FR1"),
            "cancel",
            json!({
                "id": id,
                "branchId": rpc_common::MAIN,
                "reason": "STOCK_SHORT",
                "note": "  USD ลิ้นชัก ขาด 7  "
            }),
        )
        .await;
    assert_eq!(answer.status, 200, "cancel: {}", answer.text);
    assert_eq!(
        request_reason(&counter, &id).await,
        "STOCK_SHORT | USD ลิ้นชัก ขาด 7"
    );
    counter.finish().await;
}
