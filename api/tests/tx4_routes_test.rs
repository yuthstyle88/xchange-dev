//! Ticket #672 (TX-4) — the HTTP answers of `POST /api/transactions`, `POST
//! /api/transactions/{id}/confirm` and `POST /api/transactions/{id}/cancel` through the CM-1 command
//! seam: the status, the `TransactionDto` JSON, the `Idempotency-Key`, who may call, and the refusal
//! codes. The service-level behaviour is pinned in `tx4_create_test.rs` and
//! `tx4_confirm_cancel_test.rs`; this file pins only what the wire carries.
//! Written before the implementation by the up-front test writer (claude/14cc24bd, on the xchange BE
//! lane controller claude/ad05bf8d's order, 2026-10-11). Do not edit, delete or weaken; add tests in
//! a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §4.2 (the routes, permission
//! `ExchangeTransact`, "`TransactionDto` with `status: DRAFT`", "`CONFIRMED` or `PENDING_APPROVAL`
//! (with the request id)", "`TransactionDto` `CANCELLED`", the `[required]` create input, the error
//! codes), §4.4, §4.5 (cancel); rates `rates.md` §6 (the command seam). Ticket #672 §8.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own (`tx4_common::Desk`).
//!
//! Signatures this test assumes ("choice" marks a form the design leaves open; the lane controller
//! rules on it on #672):
//! - `[required]` three `CommandAction`s registered through the commands table (`api/build.rs`), so
//!   `bootstrap::configure` serves them: `POST /api/transactions`, `POST
//!   /api/transactions/{id}/confirm`, `POST /api/transactions/{id}/cancel` (body `{ "reason" }`).
//! - `[required]` create's body is §4.2's input: `{ quoteId, customerId, bookingId?, kioskRequestId?,
//!   staffDraftId?, passport: { uploadKey }, payment: { method, reference, account, slip?: { uploadKey
//!   }, bankFeeThb, paidAt }, valueDate, notes, countConfirmed }`. `payment.method` is
//!   `PaymentMethod::as_str` (`"Cash"`). The branch is the seam's `branch` (as `POST /api/quotes`).
//! - choice: each command's `INHERITS` is `(EntryPath::Rpc, "saveTransaction")`, the row of
//!   `ExchangeTransact`, as `POST /api/quotes` (#669).
//! - choice: `TransactionDto`, camelCase, carries at least `id` (the `transactions` row's uuid),
//!   `status` (`DRAFT` | `PENDING_APPROVAL` | `CONFIRMED` | `CANCELLED`), `quoteId`,
//!   `approvalRequestId` (the `approval_requests` row's uuid while `PENDING_APPROVAL`, else `null`),
//!   `cancelReason` (`null` until cancelled) and `lines` `[{ currency, shopSide, quantity, settleThb
//!   }]`, money as a 2-place string (`"3210.00"`), as `QuoteDto` (#669).
//! - choice: refusals answer the v3 body `{ "error": { "code", … } }` with the statuses of
//!   `commands/mod.rs::map_rate_engine_error` and the seam: `validationFailed` 400, `outOfScope` 403,
//!   `staleVersion` 409, `quoteExpired` 409, `idempotencyConflict` 409, a missing `Idempotency-Key`
//!   400, signed out `signIn` 401.
//!
//! The caller is a FRONT teller of MAIN whose id is `TELLER`, the owner of the passport scan
//! `PASSPORT-MAIN` that `Desk::start` seeds (create checks the scan is the caller's, #672 Q2). The
//! customer is a DEMO one reserved at MAIN through CI-4's `CustomerService` (as `tx4_create_test.rs`),
//! so no shift is needed (§4.4 step 2) and create's passport check passes.

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration, Utc};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tx4_common::*;
use xc_auth::domain::access::Role;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
/// The caller of every test but the access ones: MAIN's FRONT teller who uploaded `PASSPORT-MAIN`.
const ME: &str = "teller";
/// CI-4 (#651): the test PII key, as `tx4_create_test.rs`.
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

/// `Desk` plus a DEMO customer reserved at MAIN; answers the customer's id.
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

// ------------------------------------------------------------ callers

/// Who is signed in, from the header `x-test-as`: `teller` (FRONT of MAIN, id `TELLER`),
/// `staff:<ROLE>:<branch>`, or absent.
struct HeaderCallers;

fn staff_roles(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown staff role {other:?}"),
    }
}

fn callers_of(spec: &str) -> Callers {
    let staff = match spec {
        "" => None,
        "teller" => Some(test_callers::staff(
            TELLER,
            &[Role::Teller],
            BranchId::new("MAIN"),
        )),
        other => {
            let mut parts = other.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {other:?}");
            let role = parts.next().expect("a role");
            let branch = parts.next().expect("a branch");
            Some(test_callers::staff(
                &format!("employee-{role}"),
                staff_roles(role),
                BranchId::new(branch),
            ))
        }
    };
    test_callers::callers(None, staff)
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

// ------------------------------------------------------------ requests

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

/// `POST uri` as `who` with `body`, sending `key` as the `Idempotency-Key` when given.
async fn post(desk: &Desk, uri: &str, who: &str, body: &Value, key: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::post()
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(body.to_string());
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

/// `POST uri` as `who` with a fresh key.
async fn call(desk: &Desk, uri: &str, who: &str, body: &Value) -> Answer {
    post(desk, uri, who, body, Some(&unique("key"))).await
}

/// §4.2's create input for quote `quote_id`, a cash payment, the seeded passport scan of MAIN.
fn create_body(quote_id: &str, customer: &str) -> Value {
    json!({
        "quoteId": quote_id,
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
    })
}

fn confirm_uri(id: &str) -> String {
    format!("/api/transactions/{id}/confirm")
}

fn cancel_uri(id: &str) -> String {
    format!("/api/transactions/{id}/cancel")
}

fn dec(v: &Value, what: &str) -> Decimal {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a decimal string, got {v}"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{what} {s:?}: {e}"))
}

/// A 2-place money string (`"3210.00"`), as the wire carries money.
fn money_str(v: &Value, what: &str) -> String {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a money string, got {v}"))
        .to_owned();
    assert!(
        s.split('.').nth(1).map(str::len) == Some(2),
        "{what}: 2 places, got {s:?}"
    );
    s
}

async fn rows(desk: &Desk) -> i64 {
    desk.db.count("SELECT count(*) FROM transactions").await
}

async fn status_of(desk: &Desk, id: &str) -> String {
    desk.db
        .text(&format!(
            "SELECT status FROM transactions WHERE id = '{id}'::uuid"
        ))
        .await
        .unwrap_or_else(|| panic!("transaction {id} is a row"))
}

/// A DRAFT created through the route; answers its id.
async fn created(desk: &Desk, cust: &str) -> String {
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let a = call(
        desk,
        "/api/transactions",
        ME,
        &create_body(&q.id.to_string(), cust),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "create: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("create answers an id: {}", a.text))
        .to_owned()
}

// ------------------------------------------------------------ create

#[actix_web::test]
async fn r1_create_answers_the_stored_draft_with_its_figures_from_the_quote() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let a = call(
        &desk,
        "/api/transactions",
        ME,
        &create_body(&q.id.to_string(), &cust),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    let t = &a.body;
    let id = t["id"].as_str().unwrap_or_else(|| panic!("id: {}", a.text));
    assert_eq!(rows(&desk).await, 1, "one transactions row");
    assert_eq!(status_of(&desk, id).await, "DRAFT", "the stored row");
    assert_eq!(t["status"], json!("DRAFT"), "status");
    assert_eq!(t["quoteId"], json!(q.id.to_string()), "quoteId");
    assert_eq!(t["approvalRequestId"], Value::Null, "approvalRequestId");
    assert_eq!(t["cancelReason"], Value::Null, "cancelReason");
    let lines = t["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines: {}", a.text));
    assert_eq!(lines.len(), 1, "one line, as the quote");
    let line = &lines[0];
    assert_eq!(line["currency"], json!("USD"), "currency");
    assert_eq!(line["shopSide"], json!("BUY"), "shopSide");
    assert_eq!(
        dec(&line["quantity"], "quantity"),
        Decimal::from(100),
        "quantity"
    );
    // §4.4: every figure is the quote's; 100 USD at 32.10, no fee
    assert_eq!(
        money_str(&line["settleThb"], "settleThb"),
        "3210.00",
        "settleThb from the quote"
    );
}

#[actix_web::test]
async fn r2_create_without_an_idempotency_key_is_refused_and_writes_nothing() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let a = post(
        &desk,
        "/api/transactions",
        ME,
        &create_body(&q.id.to_string(), &cust),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "body {}", a.text);
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

#[actix_web::test]
async fn r3_the_same_key_replays_the_same_draft_and_another_body_is_a_conflict() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let key = unique("key");
    let body = create_body(&q.id.to_string(), &cust);
    let first = post(&desk, "/api/transactions", ME, &body, Some(&key)).await;
    assert_eq!(first.status, StatusCode::OK, "first: {}", first.text);
    let again = post(&desk, "/api/transactions", ME, &body, Some(&key)).await;
    assert_eq!(again.status, StatusCode::OK, "replay: {}", again.text);
    assert_eq!(
        again.body["id"], first.body["id"],
        "the replay is the same draft"
    );
    assert_eq!(rows(&desk).await, 1, "one row after the replay");

    let mut other = body.clone();
    other["notes"] = json!("another body");
    let conflict = post(&desk, "/api/transactions", ME, &other, Some(&key)).await;
    assert_eq!(
        (conflict.status, conflict.code()),
        (StatusCode::CONFLICT, "idempotencyConflict"),
        "body {}",
        conflict.text
    );
    assert_eq!(rows(&desk).await, 1, "the conflict wrote nothing");
}

#[actix_web::test]
async fn r4_signed_out_create_is_asked_to_sign_in_and_writes_nothing() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let a = call(
        &desk,
        "/api/transactions",
        "",
        &create_body(&q.id.to_string(), &cust),
    )
    .await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::UNAUTHORIZED, "signIn"),
        "body {}",
        a.text
    );
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

#[actix_web::test]
async fn r5_create_on_an_expired_quote_answers_quote_expired() {
    let (desk, cust) = desk_with_customer().await;
    // asked an hour ago: past the 300 s default TTL (§4.3 step 5)
    let q = desk
        .quote(
            "MAIN",
            Side::Buy,
            "100",
            None,
            Utc::now() - Duration::hours(1),
        )
        .await;
    let a = call(
        &desk,
        "/api/transactions",
        ME,
        &create_body(&q.id.to_string(), &cust),
    )
    .await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "quoteExpired"),
        "body {}",
        a.text
    );
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

#[actix_web::test]
async fn r6_create_on_another_branchs_quote_answers_out_of_scope() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote(BR2, Side::Buy, "100", None, Utc::now()).await;
    let a = call(
        &desk,
        "/api/transactions",
        ME,
        &create_body(&q.id.to_string(), &cust),
    )
    .await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "body {}",
        a.text
    );
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

#[actix_web::test]
async fn r7_create_without_a_quote_id_is_a_validation_failure() {
    let (desk, cust) = desk_with_customer().await;
    let mut body = create_body("00000000-0000-4000-8000-000000000000", &cust);
    body.as_object_mut().unwrap().remove("quoteId");
    let a = call(&desk, "/api/transactions", ME, &body).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "body {}", a.text);
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

// ------------------------------------------------------------ confirm

#[actix_web::test]
async fn r8_confirm_under_the_limit_answers_confirmed_without_a_request() {
    let (desk, cust) = desk_with_customer().await;
    desk.limit("MAIN", "5000").await;
    let id = created(&desk, &cust).await;
    let a = call(&desk, &confirm_uri(&id), ME, &json!({})).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["id"], json!(id), "the same transaction");
    assert_eq!(a.body["status"], json!("CONFIRMED"), "status");
    assert_eq!(a.body["approvalRequestId"], Value::Null, "no request");
    assert_eq!(status_of(&desk, &id).await, "CONFIRMED", "the stored row");
    assert_eq!(desk.requests().await, 0, "no approval request");
}

#[actix_web::test]
async fn r9_confirm_above_the_limit_answers_pending_approval_with_the_request_id() {
    let (desk, cust) = desk_with_customer().await;
    desk.limit("MAIN", "1000").await;
    let id = created(&desk, &cust).await;
    let a = call(&desk, &confirm_uri(&id), ME, &json!({})).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["status"], json!("PENDING_APPROVAL"), "status");
    let request = a.body["approvalRequestId"]
        .as_str()
        .unwrap_or_else(|| panic!("approvalRequestId is the request's id: {}", a.text));
    let stored = desk
        .db
        .text(&format!(
            "SELECT approval_request_id::text FROM transactions WHERE id = '{id}'::uuid"
        ))
        .await;
    assert_eq!(stored.as_deref(), Some(request), "the stored request");
    assert_eq!(
        desk.db
            .count(&format!(
                "SELECT count(*) FROM approval_requests \
                 WHERE id = '{request}'::uuid AND status = 'PENDING'"
            ))
            .await,
        1,
        "the answered id is a PENDING approval request"
    );
}

#[actix_web::test]
async fn r10_confirm_of_a_confirmed_transaction_answers_stale_version() {
    let (desk, cust) = desk_with_customer().await;
    let id = created(&desk, &cust).await;
    let first = call(&desk, &confirm_uri(&id), ME, &json!({})).await;
    assert_eq!(first.status, StatusCode::OK, "first: {}", first.text);
    let again = call(&desk, &confirm_uri(&id), ME, &json!({})).await;
    assert_eq!(
        (again.status, again.code()),
        (StatusCode::CONFLICT, "staleVersion"),
        "body {}",
        again.text
    );
    assert_eq!(status_of(&desk, &id).await, "CONFIRMED", "unchanged");
}

// ------------------------------------------------------------ cancel

#[actix_web::test]
async fn r11_cancel_of_a_draft_answers_cancelled_with_the_reason() {
    let (desk, cust) = desk_with_customer().await;
    let id = created(&desk, &cust).await;
    let a = call(
        &desk,
        &cancel_uri(&id),
        ME,
        &json!({ "reason": "customer left" }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["id"], json!(id), "the same transaction");
    assert_eq!(a.body["status"], json!("CANCELLED"), "status");
    assert_eq!(
        a.body["cancelReason"],
        json!("customer left"),
        "cancelReason"
    );
    assert_eq!(status_of(&desk, &id).await, "CANCELLED", "the stored row");
}

#[actix_web::test]
async fn r12_cancel_of_a_pending_approval_answers_stale_version() {
    let (desk, cust) = desk_with_customer().await;
    desk.limit("MAIN", "1000").await;
    let id = created(&desk, &cust).await;
    let pending = call(&desk, &confirm_uri(&id), ME, &json!({})).await;
    assert_eq!(
        pending.body["status"],
        json!("PENDING_APPROVAL"),
        "premise: {}",
        pending.text
    );
    let a = call(&desk, &cancel_uri(&id), ME, &json!({ "reason": "no" })).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "staleVersion"),
        "body {}",
        a.text
    );
    assert_eq!(status_of(&desk, &id).await, "PENDING_APPROVAL", "unchanged");
}

#[actix_web::test]
async fn r13_signed_out_confirm_and_cancel_are_asked_to_sign_in() {
    let (desk, cust) = desk_with_customer().await;
    let id = created(&desk, &cust).await;
    for uri in [confirm_uri(&id), cancel_uri(&id)] {
        let a = call(&desk, &uri, "", &json!({ "reason": "x" })).await;
        assert_eq!(
            (a.status, a.code()),
            (StatusCode::UNAUTHORIZED, "signIn"),
            "{uri}: body {}",
            a.text
        );
    }
    assert_eq!(status_of(&desk, &id).await, "DRAFT", "unchanged");
}
