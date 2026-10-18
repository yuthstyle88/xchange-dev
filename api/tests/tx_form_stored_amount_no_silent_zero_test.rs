//! A stored exchange whose line quantity the form cannot hold must fail the request, never become a zero
//! amount. Written before the fix by the up-front test writer (claude/d7aa64, ticket author, on the xchange
//! lane controller's ruling: a figure computed from stored data that does not parse is `AppError::Internal`,
//! 500, never `Money::ZERO`). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The defect (`main` 104e817d): `TransactionForm::from_transaction`
//! (`crates/xc-exchange/src/domain/exchange/value_objects/transaction_form.rs:58`) builds the form's
//! `amount` through `money_of` (`:312-315`), which answers `Money::parse(..).unwrap_or(Money::ZERO)`.
//! `Money::parse` refuses anything above `Money::MAX_INPUT_UNITS` (100,000,000), while
//! `transaction_lines.quantity` is `numeric(20,4) CHECK (quantity > 0)` and `POST /api/quotes`
//! (`src/presentation/http/v3/commands/quotes.rs:139-158`) stores any positive quantity. Such a line reaches
//! the form as `0.00`:
//! - `POST /api/transactions/{id}/confirm` (`src/application/workflows/exchange/confirm.rs:134`) hands every
//!   save step's `before_in` a form whose amount is zero;
//! - `POST /api/transactions/{id}/complete` (`src/application/workflows/exchange/complete.rs:274-276`) writes
//!   the receipt from it, so the stored receipt says `Amount 0.00`, and hands every step's `after_in` the same.
//!
//! What this file requires (through the real routes, `bootstrap::configure`):
//! - a stored line of 200,000,000 units ⇒ confirm and complete answer 500 `serverError`, and nothing moves:
//!   the exchange keeps its status, no receipt is written, the drawer is untouched.
//! - keep-pins: an ordinary 100 and the boundary 100,000,000 confirm as before; an ordinary 100 completes
//!   with `Amount 100.00` on the receipt.
//!
//! The over-range line is written into the stored row with SQL so the test does not depend on how such a
//! row arrives (today `POST /api/quotes` accepts one; whether it should is a separate question).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
/// MAIN's FRONT teller, id `TELLER`, owner of the seeded scan `PASSPORT-MAIN`.
const ME: &str = "teller";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// One unit past what `Money::parse` holds (`Money::MAX_INPUT_UNITS` = 100,000,000), twice over.
const OVER_RANGE: &str = "200000000";

static PII: Once = Once::new();

// ------------------------------------------------------------ callers

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let signed_in = req.headers().get(AS).is_some();
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: if signed_in {
                    Actor::staff(TELLER, Role::Front, BranchId::new("MAIN"))
                } else {
                    None
                },
            }
        })
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

/// `POST uri` as the teller with `body` and a fresh `Idempotency-Key`; receipt files go under `files_dir`.
async fn post(pool: &sqlx::PgPool, files_dir: Option<&Path>, uri: &str, body: &Value) -> Answer {
    let settings = match files_dir {
        Some(dir) => common::settings(&[("APP_FILES__DIR", dir.to_str().expect("a UTF-8 path"))]),
        None => common::settings(&[]),
    };
    let state = AppState::new(Arc::new(settings), pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, ME))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

// ------------------------------------------------------------ fixtures

/// `Desk` plus a DEMO customer reserved at MAIN (as `tx4_routes_test.rs`); answers the customer's id.
async fn desk_with_customer() -> (Desk, String) {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        xc_customer::install_pii_keys(keys).expect("one test PII key per process");
    });
    let desk = Desk::start().await;
    let customers = CustomerService::new(BranchTx::new(Transactor::new(desk.db.pool.clone())));
    let id = customers
        .reserve(
            &BranchId::new("MAIN"),
            ReserveCustomer {
                name: "Somchai Jaidee".to_owned(),
                passport_no: format!("P{}", &uuid::Uuid::new_v4().simple().to_string()[..19])
                    .to_uppercase(),
                nationality: "THA".to_owned(),
                birth_date: String::new(),
                passport_expiry: String::new(),
                mode: "DEMO".to_owned(),
            },
            whole_second_now(),
        )
        .await
        .unwrap_or_else(|e| panic!("reserve: {e}"))
        .customer_id;
    (desk, id)
}

/// A DRAFT of USD 100 created through `POST /api/transactions`; answers its id.
async fn draft_through_the_route(desk: &Desk, customer: &str) -> String {
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let a = post(
        &desk.db.pool,
        None,
        "/api/transactions",
        &json!({
            "quoteId": q.id.to_string(),
            "customerId": customer,
            "passport": { "uploadKey": "PASSPORT-MAIN" },
            "payment": {
                "method": "Cash",
                "reference": "R-1",
                "account": "Cash drawer 1",
                "bankFeeThb": "0.00",
                "paidAt": Utc::now().to_rfc3339(),
            },
            "valueDate": "2026-10-06",
            "notes": "",
            "countConfirmed": false,
        }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "create: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("create answers an id: {}", a.text))
        .to_owned()
}

/// Sets the stored line of transaction `id` to `quantity` units (the column is `numeric(20,4) > 0`).
async fn store_quantity(db: &TestDb, id: &str, quantity: &str) {
    db.exec(&format!(
        "UPDATE transaction_lines SET quantity = {quantity} WHERE transaction_id = '{id}'::uuid"
    ))
    .await;
}

async fn status_of(db: &TestDb, id: &str) -> String {
    db.text(&format!(
        "SELECT status FROM transactions WHERE id = '{id}'::uuid"
    ))
    .await
    .unwrap_or_else(|| panic!("transaction {id} is a row"))
}

async fn receipts_of(db: &TestDb, id: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM receipts WHERE transaction_id = '{id}'::uuid"
    ))
    .await
}

/// The `Amount` cell of every receipt file written under `dir` (empty when none was written).
fn receipt_amounts(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(html) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(at) = html.find(">Amount<") {
                let rest = &html[at + ">Amount<".len()..];
                let cell = rest
                    .split('>')
                    .nth(2)
                    .and_then(|s| s.split('<').next())
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                found.push(cell);
            }
        }
    }
    found
}

/// The drawer's expected balance of `code` on MAIN's open shift, read through the ledger port (as
/// `tx5_routes_test.rs`).
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

// ------------------------------------------------------------ confirm

/// A stored line of 200,000,000 units is data the form cannot hold: confirm answers 500 and the exchange
/// stays a DRAFT. Today it confirms with a form whose amount is `0.00`.
#[actix_web::test]
async fn z1_confirm_of_a_stored_quantity_the_form_cannot_hold_is_a_server_error() {
    let (desk, cust) = desk_with_customer().await;
    let id = draft_through_the_route(&desk, &cust).await;
    store_quantity(&desk.db, &id, OVER_RANGE).await;

    let a = post(
        &desk.db.pool,
        None,
        &format!("/api/transactions/{id}/confirm"),
        &json!({}),
    )
    .await;

    assert_eq!(
        (a.status, a.code()),
        (StatusCode::INTERNAL_SERVER_ERROR, "serverError"),
        "a stored quantity of {OVER_RANGE} must fail confirm, not reach the steps as amount 0.00; body {}",
        a.text
    );
    assert_eq!(status_of(&desk.db, &id).await, "DRAFT", "nothing moved");
    assert_eq!(desk.requests().await, 0, "no approval request");
    desk.db.finish().await;
}

/// Keep-pin: an ordinary quantity confirms as before.
#[actix_web::test]
async fn z2_confirm_of_an_ordinary_quantity_still_confirms() {
    let (desk, cust) = desk_with_customer().await;
    let id = draft_through_the_route(&desk, &cust).await;

    let a = post(
        &desk.db.pool,
        None,
        &format!("/api/transactions/{id}/confirm"),
        &json!({}),
    )
    .await;

    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["status"], json!("CONFIRMED"), "{}", a.text);
    assert_eq!(status_of(&desk.db, &id).await, "CONFIRMED");
    desk.db.finish().await;
}

/// Keep-pin at the boundary: exactly `Money::MAX_INPUT_UNITS` units is a form amount, so confirm works.
#[actix_web::test]
async fn z3_confirm_at_the_largest_form_amount_still_confirms() {
    let (desk, cust) = desk_with_customer().await;
    let id = draft_through_the_route(&desk, &cust).await;
    store_quantity(&desk.db, &id, "100000000").await;

    let a = post(
        &desk.db.pool,
        None,
        &format!("/api/transactions/{id}/confirm"),
        &json!({}),
    )
    .await;

    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["status"], json!("CONFIRMED"), "{}", a.text);
    desk.db.finish().await;
}

// ------------------------------------------------------------ complete

/// A CONFIRMED exchange whose stored line is 200,000,000 units: complete answers 500, writes no receipt,
/// keeps CONFIRMED, and leaves the drawer as it was. Today it completes and the stored receipt says
/// `Amount 0.00`.
#[actix_web::test]
async fn z4_complete_of_a_stored_quantity_the_form_cannot_hold_is_a_server_error() {
    let c = Counter::start().await;
    let t = c
        .live(Side::Buy, PaymentMethod::Cash, true, whole_second_now())
        .await;
    let id = t.id.0.to_string();
    store_quantity(&c.desk.db, &id, OVER_RANGE).await;
    let uploads = c.scratch.join("uploads");

    let a = post(
        &c.desk.db.pool,
        Some(&uploads),
        &format!("/api/transactions/{id}/complete"),
        &json!({}),
    )
    .await;

    assert_eq!(
        (a.status, a.code()),
        (StatusCode::INTERNAL_SERVER_ERROR, "serverError"),
        "a stored quantity of {OVER_RANGE} must fail complete; receipt Amount cells written: {:?}; body {}",
        receipt_amounts(&uploads),
        a.text
    );
    assert_eq!(
        status_of(&c.desk.db, &id).await,
        "CONFIRMED",
        "nothing moved"
    );
    assert_eq!(receipts_of(&c.desk.db, &id).await, 0, "no receipt row");
    assert!(
        receipt_amounts(&uploads).is_empty(),
        "no receipt file: {:?}",
        receipt_amounts(&uploads)
    );
    assert_eq!(
        expected(&c, "USD").await,
        "1000.00",
        "the drawer's USD is untouched"
    );
    assert_eq!(
        expected(&c, "THB").await,
        "10000.00",
        "the drawer's THB is untouched"
    );
    c.finish().await;
}

/// Keep-pin: an ordinary quantity completes and its receipt carries the amount, not zero.
#[actix_web::test]
async fn z5_complete_of_an_ordinary_quantity_writes_its_amount_on_the_receipt() {
    let c = Counter::start().await;
    let t = c
        .live(Side::Buy, PaymentMethod::Cash, true, whole_second_now())
        .await;
    let id = t.id.0.to_string();
    let uploads = c.scratch.join("uploads");

    let a = post(
        &c.desk.db.pool,
        Some(&uploads),
        &format!("/api/transactions/{id}/complete"),
        &json!({}),
    )
    .await;

    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["status"], json!("COMPLETED"), "{}", a.text);
    assert_eq!(
        expected(&c, "USD").await,
        "1100.00",
        "the drawer took USD 100"
    );
    assert_eq!(receipts_of(&c.desk.db, &id).await, 1, "one receipt row");
    assert_eq!(
        receipt_amounts(&uploads),
        vec!["100.00".to_owned()],
        "the receipt's Amount"
    );
    c.finish().await;
}
