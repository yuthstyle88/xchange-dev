//! Ticket #6095 (F6, rehearsal finding on #564): once a kiosk request has its queue ticket, the
//! customer is told the ticket's `display_number` (`W001`), the number the board, `/queue-display`
//! and the teller call (`docs/design/blueprint/queue-tickets.md:40-43`, Q1/Q3/Q4; lane
//! controller's ruling on #6095, issuecomment-6627017842). `K-<first eight>` is v2's request label
//! and stays only for a request that has no ticket.
//!
//! Written before the implementation by the ticket author (claude/01TxaRJG). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Every answer that carries `queue` is read through the server's whole route table with real
//! sessions (`kiosk_http_common::Counter`, which opens `MAIN`'s queue, so `confirm` issues a
//! walk-in ticket): the customer's `confirm`, `status` and `customerReceipt`; the desk's `list`,
//! `get` and `notifications`; and the v3 `kiosk_queue` view. The expected number is read from
//! `queue_tickets` itself, never typed.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use actix_web::http::Method;
use kiosk_http_common::{Counter, person};
use rpc_common::Call;
use serde_json::{Value, json};

const ID: &str = "c0ffee12-0000-4000-8000-0000000f6001";
const OTHER: &str = "decaf345-0000-4000-8000-0000000f6002";

/// The accepting teller needs a `users` row and an ACTIVE counter (as `kiosk_http_test.rs`
/// `open_counters` seeds them), because `accept` starts the request's queue ticket.
async fn open_counters(pool: &sqlx::PgPool, tellers: &[&str]) {
    for teller in tellers {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             SELECT $2::uuid, organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = $1 \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("the teller's user row");
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             SELECT organization_id, id, $2::uuid, 'ACTIVE' FROM branches WHERE id = $1 \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("open the teller's counter");
    }
}

/// The `display_number` of the ticket issued for kiosk request `id`; panics if there is none.
async fn ticket_number(counter: &Counter, id: &str) -> String {
    let number: String = sqlx::query_scalar(
        "SELECT display_number FROM queue_tickets WHERE branch_id = $1 AND kiosk_request_id = $2",
    )
    .bind(rpc_common::MAIN)
    .bind(id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("request {id} has a queue ticket: {e}"));
    assert!(
        number.len() == 4
            && number.starts_with('W')
            && number[1..].chars().all(|c| c.is_ascii_digit()),
        "a walk-in ticket's number is W and three digits: {number}"
    );
    number
}

/// v2's label for a request with no ticket: `K-` and the first eight characters, upper-cased.
fn v2_label(id: &str) -> String {
    format!("K-{}", id[..8].to_uppercase())
}

fn order_no(id: &str) -> String {
    format!("ORD-{}", id.to_uppercase())
}

fn queue_of(answer: &Value) -> &str {
    answer["queue"]
        .as_str()
        .unwrap_or_else(|| panic!("`queue` is a string: {answer}"))
}

/// The row of `id` in a list of requests or queue rows.
fn row_of<'v>(rows: &'v Value, id: &str) -> &'v Value {
    rows.as_array()
        .unwrap_or_else(|| panic!("an array of rows: {rows}"))
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("{id} is listed: {rows}"))
}

/// `GET /api/v3/views/<view>?<query>` as the signed-in `who`, as a screen sends it.
async fn view(counter: &Counter, who: &str, view: &str, query: &str) -> Value {
    let mut call = Call::post(
        "http://desk.test",
        &format!("/api/v3/views/{view}?{query}"),
        Vec::<u8>::new(),
    )
    .without("x-xchange-request")
    .without("content-type")
    .without("origin");
    call.method = Method::GET;
    let call = call.cookie(
        counter
            .cookies
            .get(who)
            .unwrap_or_else(|| panic!("{who} is not signed in")),
    );
    let answer = counter.app.send(call).await;
    assert_eq!(answer.status, 200, "GET {view}: {:.400}", answer.text);
    answer.json()
}

/// A1: the customer's `confirm` answers the number of the ticket it has just issued, and the
/// request's other names are unchanged.
#[actix_web::test]
async fn confirm_answers_the_ticket_number() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    counter.hands.ids.push(ID);
    let quote = counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        )
        .await;
    assert_eq!(quote.status, 200, "quote: {}", quote.text);
    counter
        .desk
        .clock
        .set(counter.now() + chrono::Duration::seconds(4));
    let confirmed = counter
        .ask(Some("FR1"), "confirm", json!({ "id": ID }))
        .await;
    assert_eq!(confirmed.status, 200, "confirm: {}", confirmed.text);
    let number = ticket_number(&counter, ID).await;
    let body = confirmed.json();
    assert_eq!(
        queue_of(&body),
        number,
        "confirm tells the customer the ticket's number: {}",
        confirmed.text
    );
    assert_eq!(body["id"], ID, "{}", confirmed.text);
    assert_eq!(body["orderNo"], order_no(ID), "{}", confirmed.text);
    assert_eq!(body["status"], "WAITING", "{}", confirmed.text);
    counter.finish().await;
}

/// A2: what the customer screen reads afterwards (`status`, `customerReceipt`) carries the same
/// number.
#[actix_web::test]
async fn status_and_receipt_answer_the_ticket_number() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    let id = counter.waiting_request(ID).await;
    let number = ticket_number(&counter, &id).await;
    for action in ["status", "customerReceipt"] {
        let answer = counter.ask(None, action, json!({ "id": id })).await;
        assert_eq!(answer.status, 200, "{action}: {}", answer.text);
        let body = answer.json();
        assert_eq!(queue_of(&body), number, "{action}: {}", answer.text);
        assert_eq!(body["orderNo"], order_no(&id), "{action}: {}", answer.text);
    }
    counter.finish().await;
}

/// A3: the desk's compat answers (`list`, `get`, and `notifications`' working list once the
/// request is accepted) carry the ticket's number, the number the teller calls.
#[actix_web::test]
async fn the_desk_answers_carry_the_ticket_number() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    open_counters(&counter.desk.db.pool, &[&fr1.id]).await;
    let id = counter.waiting_request(ID).await;
    let number = ticket_number(&counter, &id).await;

    let list = counter.ask(Some("FR1"), "list", json!({})).await;
    assert_eq!(list.status, 200, "list: {}", list.text);
    let row = row_of(&list.json()["requests"], &id).clone();
    assert_eq!(queue_of(&row), number, "list: {}", list.text);
    assert_eq!(row["orderNo"], order_no(&id), "list: {}", list.text);

    let accepted = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(accepted.status, 200, "accept: {}", accepted.text);

    let held = counter.ask(Some("FR1"), "get", json!({ "id": id })).await;
    assert_eq!(held.status, 200, "get: {}", held.text);
    assert_eq!(queue_of(&held.json()), number, "get: {}", held.text);

    let notifications = counter.ask(Some("FR1"), "notifications", json!({})).await;
    assert_eq!(
        notifications.status, 200,
        "notifications: {}",
        notifications.text
    );
    let working = row_of(&notifications.json()["working"], &id).clone();
    assert_eq!(
        queue_of(&working),
        number,
        "notifications: {}",
        notifications.text
    );
    counter.finish().await;
}

/// A4: the v3 `kiosk_queue` view (`/kiosk-queue.html`'s `QueuePanel`) shows the ticket's number.
#[actix_web::test]
async fn the_kiosk_queue_view_shows_the_ticket_number() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    let id = counter.waiting_request(ID).await;
    let number = ticket_number(&counter, &id).await;
    let body = view(&counter, "FR1", "kiosk_queue", "state=waiting").await;
    let row = row_of(&body["data"], &id);
    assert_eq!(queue_of(row), number, "kiosk_queue: {body}");
    assert_eq!(row["orderNo"], order_no(&id), "kiosk_queue: {body}");
    counter.finish().await;
}

/// A5: a request with no ticket keeps v2's label: the quote, its `status`, and the desk's `list`
/// beside a confirmed request that does have one.
#[actix_web::test]
async fn a_request_without_a_ticket_keeps_v2s_label() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    let waiting = counter.waiting_request(ID).await;
    let number = ticket_number(&counter, &waiting).await;

    counter.hands.ids.push(OTHER);
    let quote = counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        )
        .await;
    assert_eq!(quote.status, 200, "quote: {}", quote.text);
    assert_eq!(
        queue_of(&quote.json()),
        v2_label(OTHER),
        "quote: {}",
        quote.text
    );
    let none: Option<String> = sqlx::query_scalar(
        "SELECT display_number FROM queue_tickets WHERE branch_id = $1 AND kiosk_request_id = $2",
    )
    .bind(rpc_common::MAIN)
    .bind(OTHER)
    .fetch_optional(&counter.desk.db.pool)
    .await
    .expect("read queue_tickets");
    assert_eq!(none, None, "a quoted request has no ticket");

    let status = counter.ask(None, "status", json!({ "id": OTHER })).await;
    assert_eq!(status.status, 200, "status: {}", status.text);
    assert_eq!(
        queue_of(&status.json()),
        v2_label(OTHER),
        "status: {}",
        status.text
    );

    let list = counter.ask(Some("FR1"), "list", json!({})).await;
    assert_eq!(list.status, 200, "list: {}", list.text);
    assert_eq!(
        queue_of(row_of(&list.json()["requests"], &waiting)),
        number,
        "list: the confirmed request: {}",
        list.text
    );
    counter.finish().await;
}

/// A6: a ticket that has ended still names the request: after the desk cancels a WAITING request
/// (its ticket is ended with it), `status` answers the number that was called, not `K-`.
#[actix_web::test]
async fn a_cancelled_request_keeps_its_ticket_number() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    let id = counter.waiting_request(ID).await;
    let number = ticket_number(&counter, &id).await;
    let cancelled = counter
        .ask(
            Some("FR1"),
            "cancel",
            json!({ "id": id, "branchId": rpc_common::MAIN, "reason": "CUSTOMER_LEFT" }),
        )
        .await;
    assert_eq!(cancelled.status, 200, "cancel: {}", cancelled.text);
    let body = cancelled.json();
    assert_eq!(body["status"], "CANCELLED", "cancel: {}", cancelled.text);
    assert_eq!(queue_of(&body), number, "cancel: {}", cancelled.text);
    let status = counter.ask(None, "status", json!({ "id": id })).await;
    assert_eq!(status.status, 200, "status: {}", status.text);
    assert_eq!(queue_of(&status.json()), number, "status: {}", status.text);
    counter.finish().await;
}
