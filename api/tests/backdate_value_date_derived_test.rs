//! F44 up-front tests (`docs/design/backdated-keying.md` §15.1, §15.3 rows F44-1..F44-4; day 4's keying on `#564`
//! issuecomment-6677985090): `valueDate` is optional on `POST /api/transactions/backdated`. When it is absent or
//! `null`, the server dates the deal with `ShiftPlan::business_date_of(paidAt)` under the branch's plan. A sent
//! `valueDate` is still checked as before (CS-20b). Written before the implementation by the sub-controller
//! claude/3b81c9d5. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The harness is `backdate_http_test.rs`'s, reduced to the Owner. MAIN keeps the migration's default plan
//! (A 06–14, B 14–22, C 22–06, cutover 06:00, Asia/Bangkok). No shift is open. MAIN's SAFE is funded through the ledger.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx4_*` test does.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::lg4_fund;
use xc_cash::domain::posting::CashPlace;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const OWNER: &str = "owner-f44";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// 2026-11-04 13:23 ICT: a day row.
const DAY_PAID_AT: &str = "2026-11-04T13:23:00+07:00";
/// 2026-11-05 02:59 ICT: a night row, before the 06:00 cutover, so on business date 2026-11-04.
const NIGHT_PAID_AT: &str = "2026-11-05T02:59:00+07:00";

static PII: Once = Once::new();

struct OwnerSession;

impl SessionReader for OwnerSession {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let callers = test_callers::callers(Some(test_callers::owner(OWNER)), None);
        Box::pin(async move { callers })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

struct F44 {
    desk: Desk,
    scratch: std::path::PathBuf,
    live: String,
}

async fn start() -> F44 {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        let _ = xc_customer::install_pii_keys(keys);
    });
    let desk = Desk::start().await;
    let live = CustomerService::new(BranchTx::new(Transactor::new(desk.db.pool.clone())))
        .reserve(
            &BranchId::new("MAIN"),
            ReserveCustomer {
                name: "Somchai Jaidee".to_owned(),
                passport_no: format!("P{}", &uuid::Uuid::new_v4().simple().to_string()[..19])
                    .to_uppercase(),
                nationality: "THA".to_owned(),
                birth_date: String::new(),
                passport_expiry: String::new(),
                mode: "LIVE".to_owned(),
            },
            whole_second_now(),
        )
        .await
        .expect("reserve a LIVE customer")
        .customer_id;
    lg4_fund(
        &desk.db.pool,
        CashPlace::Bank,
        CashPlace::Safe(BranchId::new("MAIN")),
        "THB",
        "100000.00",
    )
    .await;
    let scratch = std::env::temp_dir().join(unique("xc-f44"));
    std::fs::create_dir_all(&scratch).unwrap();
    F44 {
        desk,
        scratch,
        live,
    }
}

/// BUY 100 USD cash at 33.53 (no passport: §11 Q1 = B), paid at `paid_at`, with no `valueDate` key.
fn body(customer: &str, paid_at: &str) -> Value {
    json!({
        "customerId": customer,
        "side": "BUY",
        "currency": "USD",
        "quantity": "100",
        "rate": "33.53",
        "payment": {
            "method": "Cash",
            "reference": "",
            "account": "",
            "bankFeeThb": "0.00",
            "paidAt": paid_at,
        },
        "notes": "F44",
    })
}

async fn post(desk: &Desk, scratch: &std::path::Path, body: &Value) -> Answer {
    let dir = scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        desk.db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(OwnerSession);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri("/api/transactions/backdated")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, "owner"))
        .insert_header(("idempotency-key", unique("f44")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The stored `value_date` of the answered row.
async fn stored_value_date(desk: &Desk, a: &Answer) -> Option<String> {
    let id = a.body["id"].as_str().expect("an id");
    desk.db
        .text(&format!(
            "SELECT value_date::text FROM transactions WHERE id = '{id}'"
        ))
        .await
}

async fn finish(desk: Desk, scratch: std::path::PathBuf) {
    let _ = std::fs::remove_dir_all(&scratch);
    desk.db.finish().await;
}

/// F44-1: a day row with no `valueDate` is dated on its own day.
#[actix_web::test]
async fn f44_1_a_day_row_without_value_date_is_dated_its_business_date() {
    let F44 {
        desk,
        scratch,
        live,
    } = start().await;
    let a = post(&desk, &scratch, &body(&live, DAY_PAID_AT)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["status"], "COMPLETED", "{}", a.text);
    assert_eq!(
        stored_value_date(&desk, &a).await.as_deref(),
        Some("2026-11-04"),
        "{}",
        a.text
    );
    finish(desk, scratch).await;
}

/// F44-2: a 02:59 row with no `valueDate` is dated the previous business date (cutover 06:00), not the calendar date.
#[actix_web::test]
async fn f44_2_a_night_row_without_value_date_is_dated_the_previous_business_date() {
    let F44 {
        desk,
        scratch,
        live,
    } = start().await;
    let a = post(&desk, &scratch, &body(&live, NIGHT_PAID_AT)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        stored_value_date(&desk, &a).await.as_deref(),
        Some("2026-11-04"),
        "02:59 on 5 Nov belongs to business date 4 Nov: {}",
        a.text
    );
    finish(desk, scratch).await;
}

/// F44-3: `valueDate: null` reads as absent.
#[actix_web::test]
async fn f44_3_a_null_value_date_reads_as_absent() {
    let F44 {
        desk,
        scratch,
        live,
    } = start().await;
    let mut b = body(&live, NIGHT_PAID_AT);
    b["valueDate"] = Value::Null;
    let a = post(&desk, &scratch, &b).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        stored_value_date(&desk, &a).await.as_deref(),
        Some("2026-11-04"),
        "{}",
        a.text
    );
    finish(desk, scratch).await;
}

/// F44-4: a sent `valueDate` that is the calendar date, not the business date, is still refused (CS-20b).
#[actix_web::test]
async fn f44_4_a_sent_calendar_date_is_still_refused() {
    let F44 {
        desk,
        scratch,
        live,
    } = start().await;
    let mut b = body(&live, NIGHT_PAID_AT);
    b["valueDate"] = json!("2026-11-05");
    let a = post(&desk, &scratch, &b).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.body["error"]["code"], "validationFailed", "{}", a.text);
    assert_eq!(a.body["error"]["field"], "valueDate", "{}", a.text);
    assert_eq!(
        desk.db
            .count("SELECT count(*) FROM transactions WHERE backdated")
            .await,
        0,
        "nothing written"
    );
    finish(desk, scratch).await;
}
