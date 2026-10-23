//! Ticket #5021 — v3 create and complete refusal codes:
//! - RC1: POST /api/transactions with LIVE customer and no open shift returns 409 noOpenShift
//! - RC2: POST /api/transactions/{id}/complete with unconfirmed cash returns 409 countFirst

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{header, StatusCode};
use actix_web::{test as atest, web, App, HttpRequest};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::{json, Value};
use tx4_common::*;
use tx5_common::*;
use xc_auth::domain::access::Role;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::TransactionStatus;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::devices::device_service::COUNT_FIRST;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ME: &str = "teller";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        let staff = match spec.as_str() {
            "" => None,
            "teller" => Some(test_callers::staff(
                TELLER,
                &[Role::Teller],
                BranchId::new("MAIN"),
            )),
            other => panic!("unexpected caller {other:?}"),
        };
        Box::pin(async move { test_callers::callers(None, staff) })
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

    fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

async fn create_route(desk: &Desk, body: &Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
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

async fn complete_route(c: &Counter, id: &str) -> Answer {
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
    let req = atest::TestRequest::post()
        .uri(&format!("/api/transactions/{id}/complete"))
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, ME))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(json!({}).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

#[actix_web::test]
async fn rc1_v3_create_live_no_open_shift_answers_409_no_open_shift() {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        let _ = xc_customer::install_pii_keys(keys);
    });
    let desk = Desk::start().await;
    let customers = CustomerService::new(BranchTx::new(Transactor::new(desk.db.pool.clone())));
    let customer_id = customers
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
        .unwrap_or_else(|e| panic!("reserve: {e}"))
        .customer_id;

    let q = desk.quote("MAIN", Side::Buy, "100", None, Utc::now()).await;
    let body = json!({
        "quoteId": q.id.to_string(),
        "customerId": customer_id,
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
    });

    let a = create_route(&desk, &body).await;
    assert_eq!(a.status, StatusCode::CONFLICT, "{}", a.text);
    assert_eq!(a.code(), "noOpenShift", "{}", a.text);
    assert_eq!(a.message(), ShiftError::NoOpenShift.to_string(), "{}", a.text);

    assert_eq!(
        desk.db.count("SELECT count(*) FROM transactions").await,
        0,
        "nothing is written"
    );
    desk.db.finish().await;
}

#[actix_web::test]
async fn rc2_v3_complete_live_unconfirmed_cash_answers_409_count_first() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, false, now).await;

    let a = complete_route(&c, &t.id.0.to_string()).await;
    assert_eq!(a.status, StatusCode::CONFLICT, "{}", a.text);
    assert_eq!(a.code(), "countFirst", "{}", a.text);
    assert_eq!(a.message(), COUNT_FIRST, "{}", a.text);

    assert_eq!(
        c.desk.stored(t.id).await.status,
        TransactionStatus::Confirmed,
        "transaction stays CONFIRMED"
    );
    c.finish().await;
}
