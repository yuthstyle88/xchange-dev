//! v3 `POST /api/transactions` refuses what the compat save refuses, before anything is stored:
//! a non-cash payment without its reference or slip (K1), a `bookingId` that names no booking (K2),
//! and a quote another unfinished exchange already names (K3). Guards: a retry with the same
//! `Idempotency-Key` still replays the first draft (K3b), a quote whose exchange is CONFIRMED is
//! still refused as today (K3c), and a Cash payment needs neither reference nor slip (K4).
//! Written before the implementation by the up-front test writer for the xchange BE lane controller
//! claude/7ca80c, 2026-10-24. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Defects at origin/main 3bac87e0a:
//! - K1: `create.rs` step 3 checks the slip only when one is sent and never looks at the reference;
//!   the route (`commands/transactions.rs`) bounds the reference to 0..=100 characters only. The
//!   compat form refuses both (`transaction_form.rs` step 19 `NO_PAYMENT_REF`, then step 23
//!   `FILE_MISSING`); design `blueprint/quote-transaction.md` §3 `transaction_payments.reference`
//!   and `slip_file_key`: *"required unless Cash"*. Today both bodies are answered 200 DRAFT.
//! - K2: create never reads the booking; `transactions.booking_id` references `bookings(id)`
//!   (`migrations/20261218120000_retire_v2_bookings.sql`), so an unknown id fails at the insert
//!   and is answered 500 `serverError`.
//! - K3: create checks only that the quote is ACTIVE; the quote is accepted at confirm, so while
//!   the first exchange is DRAFT a second create with another key makes a second DRAFT of the same
//!   quote. (Once the first is CONFIRMED or PENDING_APPROVAL the quote is ACCEPTED and create
//!   already answers `staleVersion`, which K3c pins.)
//!
//! What these tests ask of the fix:
//! - K1: 400 `validationFailed` whose message is today's compat sentence, the reference checked
//!   first: no reference ⇒ `ต้องระบุเลขอ้างอิงการโอน`; a reference but no slip ⇒ `แนบเอกสารไม่ครบ`.
//! - K2: 400 `validationFailed` with `field: "bookingId"`.
//! - K3: 409 `staleVersion`.
//! - Every refusal stores no `transactions` row.
//!
//! Harness: `tx4_routes_test.rs`'s (the CM-1 command seam served by `bootstrap::configure`, the
//! caller MAIN's FRONT teller `TELLER`, a DEMO customer so no shift is needed). Needs
//! `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use quote_common::*;
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
/// CI-4 (#651): the test PII key, as `tx4_routes_test.rs`.
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// Today's compat sentences (`transaction_form.rs` `NO_PAYMENT_REF`, `FILE_MISSING`).
const NO_PAYMENT_REF: &str = "ต้องระบุเลขอ้างอิงการโอน";
const FILE_MISSING: &str = "แนบเอกสารไม่ครบ";

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

/// MAIN's FRONT teller `TELLER` when the header `x-test-as` is `teller`; signed out otherwise.
struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let teller = req
            .headers()
            .get(AS)
            .is_some_and(|v| v.as_bytes() == b"teller");
        Box::pin(async move {
            let staff =
                teller.then(|| test_callers::staff(TELLER, &[Role::Teller], BranchId::new("MAIN")));
            test_callers::callers(None, staff)
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

    fn field(&self) -> &str {
        self.body["error"]["field"].as_str().unwrap_or("")
    }

    fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

/// `POST /api/transactions` as the teller with `body` and the `Idempotency-Key` `key`.
async fn create(desk: &Desk, body: &Value, key: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri("/api/transactions")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, "teller"))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// `POST /api/transactions` with a fresh key.
async fn create_fresh(desk: &Desk, body: &Value) -> Answer {
    create(desk, body, &unique("key")).await
}

/// `POST /api/transactions/{id}/confirm` as the teller with a fresh key.
async fn confirm(desk: &Desk, id: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(&format!("/api/transactions/{id}/confirm"))
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, "teller"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload("{}");
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// §4.2's create input for `quote_id` with `payment`, and MAIN's seeded passport scan.
fn body_with(quote_id: &str, customer: &str, payment: Value) -> Value {
    json!({
        "quoteId": quote_id,
        "customerId": customer,
        "passport": { "uploadKey": "PASSPORT-MAIN" },
        "payment": payment,
        "valueDate": "2026-10-06",
        "notes": "",
        "countConfirmed": false,
    })
}

/// A cash payment with a reference, as `tx4_routes_test.rs::create_body`.
fn cash() -> Value {
    json!({
        "method": "Cash",
        "reference": "R-1",
        "account": "Cash drawer 1",
        "bankFeeThb": "0.00",
        "paidAt": Utc::now().to_rfc3339(),
    })
}

async fn rows(desk: &Desk) -> i64 {
    desk.db.count("SELECT count(*) FROM transactions").await
}

/// A file of the teller at MAIN (a `files` row), so a slip naming it passes the attachment check.
async fn slip_of_main(desk: &Desk) -> String {
    let key = unique("SLIP-MAIN");
    desk.db
        .exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('{key}', 'MAIN', '{TELLER}', 'application/pdf', 'sha-{key}', now())"
        ))
        .await;
    key
}

// ------------------------------------------------------------ K1 non-cash needs reference + slip

#[actix_web::test]
async fn k1a_a_bank_transfer_without_a_reference_is_refused_with_todays_sentence() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let mut payment = cash();
    payment["method"] = json!("Bank Transfer");
    payment["reference"] = json!("");
    let a = create_fresh(&desk, &body_with(&q.id.to_string(), &cust, payment)).await;
    assert_eq!(
        (a.status, a.code(), a.message()),
        (StatusCode::BAD_REQUEST, "validationFailed", NO_PAYMENT_REF),
        "K1: a non-cash payment needs its reference first (compat step 19); body {}",
        a.text
    );
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

#[actix_web::test]
async fn k1b_a_bank_transfer_with_a_reference_but_no_slip_is_refused_with_todays_sentence() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let mut payment = cash();
    payment["method"] = json!("Bank Transfer");
    let a = create_fresh(&desk, &body_with(&q.id.to_string(), &cust, payment)).await;
    assert_eq!(
        (a.status, a.code(), a.message()),
        (StatusCode::BAD_REQUEST, "validationFailed", FILE_MISSING),
        "K1: a non-cash payment needs its slip (compat step 23); body {}",
        a.text
    );
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

#[actix_web::test]
async fn k1c_a_bank_transfer_with_its_reference_and_slip_is_accepted() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let slip = slip_of_main(&desk).await;
    let mut payment = cash();
    payment["method"] = json!("Bank Transfer");
    payment["slip"] = json!({ "uploadKey": slip });
    let a = create_fresh(&desk, &body_with(&q.id.to_string(), &cust, payment)).await;
    assert_eq!(a.status, StatusCode::OK, "guard: body {}", a.text);
    assert_eq!(a.body["status"], json!("DRAFT"), "guard: a DRAFT");
    assert_eq!(rows(&desk).await, 1, "one row");
}

// ------------------------------------------------------------ K2 unknown booking

#[actix_web::test]
async fn k2_a_booking_id_that_names_no_booking_is_a_validation_failure_on_booking_id() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let mut body = body_with(&q.id.to_string(), &cust, cash());
    body["bookingId"] = json!("BK-nope");
    let a = create_fresh(&desk, &body).await;
    assert_eq!(
        (a.status, a.code(), a.field()),
        (StatusCode::BAD_REQUEST, "validationFailed", "bookingId"),
        "K2: create reads the booking before the insert; body {}",
        a.text
    );
    assert_eq!(rows(&desk).await, 0, "nothing stored");
}

// ------------------------------------------------------------ K3 one unfinished exchange per quote

#[actix_web::test]
async fn k3_a_second_create_of_a_quote_whose_draft_is_unfinished_is_stale_and_stores_nothing() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let body = body_with(&q.id.to_string(), &cust, cash());
    let first = create_fresh(&desk, &body).await;
    assert_eq!(first.status, StatusCode::OK, "premise: {}", first.text);
    assert_eq!(first.body["status"], json!("DRAFT"), "premise: a DRAFT");

    let second = create_fresh(&desk, &body).await;
    assert_eq!(
        (second.status, second.code()),
        (StatusCode::CONFLICT, "staleVersion"),
        "K3: draft {} already names quote {}; body {}",
        first.body["id"],
        q.id,
        second.text
    );
    assert_eq!(rows(&desk).await, 1, "only the first draft is stored");
}

#[actix_web::test]
async fn k3b_a_retry_with_the_same_key_still_replays_the_first_draft() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let body = body_with(&q.id.to_string(), &cust, cash());
    let key = unique("key");
    let first = create(&desk, &body, &key).await;
    assert_eq!(first.status, StatusCode::OK, "first: {}", first.text);
    let again = create(&desk, &body, &key).await;
    assert_eq!(again.status, StatusCode::OK, "guard: replay {}", again.text);
    assert_eq!(
        again.body["id"], first.body["id"],
        "guard: the replay is the first draft"
    );
    assert_eq!(rows(&desk).await, 1, "one row after the replay");
}

#[actix_web::test]
async fn k3c_a_second_create_of_a_quote_whose_exchange_is_confirmed_is_stale() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let body = body_with(&q.id.to_string(), &cust, cash());
    let first = create_fresh(&desk, &body).await;
    assert_eq!(first.status, StatusCode::OK, "premise: {}", first.text);
    let id = first.body["id"].as_str().expect("an id").to_owned();
    let confirmed = confirm(&desk, &id).await;
    assert_eq!(
        confirmed.body["status"],
        json!("CONFIRMED"),
        "premise: {}",
        confirmed.text
    );

    let second = create_fresh(&desk, &body).await;
    assert_eq!(
        (second.status, second.code()),
        (StatusCode::CONFLICT, "staleVersion"),
        "guard: body {}",
        second.text
    );
    assert_eq!(rows(&desk).await, 1, "only the confirmed exchange");
}

// ------------------------------------------------------------ K4 cash needs neither

#[actix_web::test]
async fn k4_a_cash_payment_without_a_reference_or_slip_is_accepted() {
    let (desk, cust) = desk_with_customer().await;
    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let mut payment = cash();
    payment["reference"] = json!("");
    let a = create_fresh(&desk, &body_with(&q.id.to_string(), &cust, payment)).await;
    assert_eq!(a.status, StatusCode::OK, "guard: body {}", a.text);
    assert_eq!(a.body["status"], json!("DRAFT"), "guard: a DRAFT");
    assert_eq!(rows(&desk).await, 1, "one row");
}
