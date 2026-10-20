//! Ticket #873 (TX-5) — the HTTP answers of `POST /api/transactions/{id}/complete` through the CM-1 command seam:
//! the answer, the `Idempotency-Key` replay (a repeat posts once), who may call, the refusal codes, and the stock
//! change the route's posting makes. The service-level behaviour is pinned in `tx5_complete_test.rs`; this file
//! pins what the wire carries.
//! Written before the implementation by the up-front test writer (claude/07d3765f, on the xchange BE lane
//! controller claude/ad05bf8d's order, 2026-10-11). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §4.2, §4.5, §6.1, §9.1 TX-5; #873 §8.
//!
//! Signatures this test assumes ("choice" marks a form the design leaves open; the lane controller rules on #873):
//! - `[required]` a `CommandAction` registered through the commands table (`api/build.rs`), so `bootstrap::configure`
//!   serves `POST /api/transactions/{id}/complete` (#873 §8), body `{}`, with `Idempotency-Key`.
//! - choice: its `INHERITS` is `(EntryPath::Rpc, "saveTransaction")`, as TX-4's confirm and cancel.
//! - choice: the answer is TX-4's `TransactionDto` (camelCase, `status`, `documentRef`) with a `receipt` object
//!   `{ number, revision }` beside it (#873 §8 "`TransactionDto` `COMPLETED` + `ReceiptDto`").
//! - choice: refusals answer `{ "error": { "code" } }` with TX-4's statuses (`commands/transactions.rs::refusal`):
//!   `staleVersion` 409, `quoteExpired` 409, `outOfScope` 403; a missing key 400; signed out `signIn` 401.
//! - `[required]` (design §6.1, #873 Q9 ruling issuecomment-6107639056): until LG-4's cutover (#929) the route
//!   posted through the interim port, which moved the drawer for an `EXCHANGE` source as today's save does
//!   (`ShiftService::apply_delta_in`): it refuses below zero with `stockInsufficient` (409, mapped in TX-5's
//!   `commands/complete.rs`).
//!
//! The caller is MAIN's FRONT teller (`TELLER`). The exchange is a CONFIRMED LIVE one on MAIN's open shift
//! (`tx5_common::Counter`), made through the services, so only `complete` goes over the wire.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::ExchangeTransaction;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ME: &str = "teller";

/// Who is signed in, from `x-test-as`: `teller` (FRONT of MAIN, id `TELLER`), `staff:<ROLE>:<branch>`, or absent.
struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    let staff = match spec {
        "" => None,
        "teller" => Actor::staff(TELLER, Role::Front, BranchId::new("MAIN")),
        other => {
            let mut parts = other.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {other:?}");
            let role = Role::parse(parts.next().expect("a role")).expect("a known role");
            let branch = parts.next().expect("a branch");
            Actor::staff(
                format!("employee-{}", role.as_str()),
                role,
                BranchId::new(branch),
            )
        }
    };
    Sessions { owner: None, staff }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

/// `POST /api/transactions/{id}/complete` as `who`, with `key` as the `Idempotency-Key` when given. The receipt
/// files go to the counter's scratch directory.
async fn complete(c: &Counter, id: &str, who: &str, key: Option<&str>) -> Answer {
    let dir = c.scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        c.desk.db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::post()
        .uri(&format!("/api/transactions/{id}/complete"))
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(json!({}).to_string());
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(key) = key {
        req = req.insert_header(("idempotency-key", key));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn id_of(t: &ExchangeTransaction) -> String {
    t.id.0.to_string()
}

/// The drawer's expected balance of `code` on MAIN's open shift. LG-4 (#929, 1:1): read through the port,
/// `CashPostings::expected_in(Drawer(shift))`.
async fn expected(c: &Counter, code: &str) -> String {
    use xc_cash::domain::posting::{CashPlace, CashPostings};
    let mut tx = c.desk.db.pool.begin().await.expect("begin");
    let b = xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .expected_in(&mut tx, &CashPlace::Drawer(c.shift.clone()))
        .await
        .expect("the drawer through the port");
    tx.rollback().await.expect("rollback");
    let m = b.get(xc_kernel::money::CurrencyCode::parse(code).expect("a currency"));
    format!(
        "{:.2}",
        rust_decimal::Decimal::new(m.try_minor().unwrap(), 2)
    )
}

#[actix_web::test]
async fn r1_complete_answers_completed_with_its_document_ref_and_receipt_revision_1() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;

    let a = complete(&c, &id_of(&t), ME, Some(&unique("key"))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["id"], json!(id_of(&t)));
    assert_eq!(a.body["status"], "COMPLETED", "{}", a.text);
    let document_ref = a.body["documentRef"]
        .as_str()
        .unwrap_or_else(|| panic!("a documentRef: {}", a.text))
        .to_owned();
    assert_eq!(
        c.desk.stored(t.id).await.document_ref.as_deref(),
        Some(document_ref.as_str())
    );
    assert_eq!(a.body["receipt"]["revision"], 1, "{}", a.text);
    assert_eq!(a.body["receipt"]["number"], json!(document_ref));
    c.finish().await;
}

/// §9.1: a repeat with the same key replays the answer and posts once.
#[actix_web::test]
async fn r2_the_same_key_replays_the_answer_and_posts_once() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let key = unique("key");

    let first = complete(&c, &id_of(&t), ME, Some(&key)).await;
    assert_eq!(first.status, StatusCode::OK, "first {}", first.text);
    assert_eq!(expected(&c, "USD").await, "1100.00", "posted once");

    let again = complete(&c, &id_of(&t), ME, Some(&key)).await;
    assert_eq!(again.status, StatusCode::OK, "replay {}", again.text);
    assert_eq!(again.body, first.body, "the same answer");
    assert_eq!(expected(&c, "USD").await, "1100.00", "nothing posted again");
    assert_eq!(expected(&c, "THB").await, "6790.00", "nothing posted again");
    assert_eq!(
        c.desk
            .db
            .count(&format!(
                "SELECT count(*) FROM receipts WHERE transaction_id = '{}'",
                t.id.0
            ))
            .await,
        1,
        "one receipt"
    );
    c.finish().await;
}

/// §6.1: the route's posting changes the drawer as today's save does. A cash BUY of USD 100 at 32.10 adds
/// USD 100.00 and takes THB 3,210.00 (opening THB 10,000.00, USD 1,000.00).
#[actix_web::test]
async fn r3_a_live_cash_buy_moves_the_drawer_as_todays_save_does() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    assert_eq!(t.lines[0].settle_thb.to_string(), "3210.00");

    let a = complete(&c, &id_of(&t), ME, Some(&unique("key"))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(expected(&c, "USD").await, "1100.00");
    assert_eq!(expected(&c, "THB").await, "6790.00");
    c.finish().await;
}

/// A non-cash payment leaves the drawer's baht alone (today's `StockDelta::for_tx(.., cash: false)`).
#[actix_web::test]
async fn r4_a_live_transfer_buy_leaves_the_drawers_baht_alone() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c
        .live(Side::Buy, PaymentMethod::BankTransfer, false, now)
        .await;

    let a = complete(&c, &id_of(&t), ME, Some(&unique("key"))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(expected(&c, "USD").await, "1100.00");
    assert_eq!(expected(&c, "THB").await, "10000.00");
    c.finish().await;
}

#[actix_web::test]
async fn r5_complete_without_an_idempotency_key_is_refused_and_writes_nothing() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;

    let a = complete(&c, &id_of(&t), ME, None).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "body {}", a.text);
    assert_eq!(c.desk.stored(t.id).await.document_ref, None);
    assert_eq!(
        expected(&c, "USD").await,
        "1000.00",
        "the drawer did not move"
    );
    c.finish().await;
}

#[actix_web::test]
async fn r6_signed_out_complete_is_asked_to_sign_in() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;

    let a = complete(&c, &id_of(&t), "", Some(&unique("key"))).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::UNAUTHORIZED, "signIn"),
        "body {}",
        a.text
    );
    assert_eq!(
        expected(&c, "USD").await,
        "1000.00",
        "the drawer did not move"
    );
    c.finish().await;
}

#[actix_web::test]
async fn r7_a_completed_exchange_answers_stale_version() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let first = complete(&c, &id_of(&t), ME, Some(&unique("key"))).await;
    assert_eq!(first.status, StatusCode::OK, "first {}", first.text);

    let again = complete(&c, &id_of(&t), ME, Some(&unique("key"))).await;
    assert_eq!(
        (again.status, again.code()),
        (StatusCode::CONFLICT, "staleVersion"),
        "body {}",
        again.text
    );
    assert_eq!(expected(&c, "USD").await, "1100.00", "posted once");
    c.finish().await;
}

#[actix_web::test]
async fn r8_a_quote_past_its_expiry_answers_quote_expired() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    // a new publication ends the quote now (D-5), as the clock cannot be moved over HTTP
    c.desk.db.publish("MAIN", &[USD]).await;

    let a = complete(&c, &id_of(&t), ME, Some(&unique("key"))).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "quoteExpired"),
        "body {}",
        a.text
    );
    assert_eq!(
        expected(&c, "USD").await,
        "1000.00",
        "the drawer did not move"
    );
    c.finish().await;
}

#[actix_web::test]
async fn r9_a_teller_of_another_branch_is_out_of_scope() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;

    let a = complete(
        &c,
        &id_of(&t),
        &format!("staff:FRONT:{BR2}"),
        Some(&unique("key")),
    )
    .await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "body {}",
        a.text
    );
    assert_eq!(
        expected(&c, "USD").await,
        "1000.00",
        "the drawer did not move"
    );
    assert_eq!(c.desk.stored(t.id).await.document_ref, None);
    c.finish().await;
}

/// §9.1 / Q9: a cash SELL beyond the drawer's USD (1,000.00 at opening) answers 409 `stockInsufficient` and moves
/// nothing.
#[actix_web::test]
async fn r10_a_sell_beyond_the_drawer_answers_stock_insufficient_and_moves_nothing() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "2000", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;

    let a = complete(&c, &id_of(&t), ME, Some(&unique("key"))).await;
    assert_eq!(
        (a.status, a.code()),
        // re-expected for #3405 (owner: negative stock waits for approval)
        (StatusCode::CONFLICT, "approvalRequired"),
        "body {}",
        a.text
    );
    assert_eq!(expected(&c, "USD").await, "1000.00");
    assert_eq!(expected(&c, "THB").await, "10000.00");
    assert_eq!(
        c.desk.stored(t.id).await.document_ref,
        None,
        "not completed"
    );
    c.finish().await;
}
