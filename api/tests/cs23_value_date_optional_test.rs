//! Up-front tests for ticket CS-23 (#6192): `valueDate` optional on `POST /api/transactions`.
//!
//! Behaviour:
//! - V1: DEMO sale without `valueDate` (defaults 06/14/22) -> stored `value_date` = the business date of now under the defaults
//! - V2: DEMO sale with `valueDate 2026-10-06` -> stored 2026-10-06 (unchanged)
//! - V3: `valueDate "06-10-2026"` -> `invalid valueDate` (unchanged)
//! - V4: LIVE sale without `valueDate` -> accepted; stored = its shift's business date

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
use xc_cash::domain::posting::CashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

fn init_pii() {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        let _ = xc_customer::install_pii_keys(keys);
    });
}

struct HeaderCallers {
    teller_id: String,
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        let staff = match spec.as_str() {
            "" => None,
            "teller" => Some(test_callers::staff(
                &self.teller_id,
                &[Role::Teller],
                BranchId::new("MAIN"),
            )),
            other => panic!("unknown caller {other:?}"),
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
}

async fn seed_teller_and_passport(desk: &Desk) -> String {
    let user_id: uuid::Uuid = if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT u.id FROM users u JOIN organizations o ON o.id = u.organization_id \
         WHERE o.code = 'XC' AND u.name = 'test' AND u.created_by = 'test' \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_optional(&desk.db.pool)
    .await
    .unwrap()
    {
        id
    } else {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
             RETURNING id",
        )
        .fetch_one(&desk.db.pool)
        .await
        .unwrap()
    };
    let user_str = user_id.to_string();
    desk.db
        .exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('PASSPORT-MAIN', 'MAIN', '{user_str}', 'image/jpeg', 'sha-MAIN', now()) \
             ON CONFLICT (key) DO UPDATE SET owner_id = '{user_str}'"
        ))
        .await;
    user_str
}

async fn reserve_customer(desk: &Desk, mode: &str) -> String {
    init_pii();
    let customers = CustomerService::new(BranchTx::new(Transactor::new(desk.db.pool.clone())));
    customers
        .reserve(
            &BranchId::new("MAIN"),
            ReserveCustomer {
                name: "Somchai Jaidee".to_owned(),
                passport_no: format!("P{}", &uuid::Uuid::new_v4().simple().to_string()[..19])
                    .to_uppercase(),
                nationality: "THA".to_owned(),
                birth_date: String::new(),
                passport_expiry: String::new(),
                mode: mode.to_owned(),
            },
            whole_second_now(),
        )
        .await
        .unwrap_or_else(|e| panic!("reserve: {e}"))
        .customer_id
}

async fn post_transaction(desk: &Desk, teller_id: &str, body: &Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers {
        teller_id: teller_id.to_owned(),
    });
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
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn rows(desk: &Desk) -> i64 {
    desk.db.count("SELECT count(*) FROM transactions").await
}

async fn stored_value_date(desk: &Desk, id: &str) -> String {
    desk.db
        .text(&format!(
            "SELECT value_date::text FROM transactions WHERE id = '{id}'::uuid"
        ))
        .await
        .unwrap_or_else(|| panic!("transaction {id} is a row"))
}

async fn open_shift_for_teller(desk: &Desk, teller_id: &str, business_date: &str) -> String {
    let shift_id = format!("SH-{}", uuid::Uuid::new_v4());
    let teller_uuid = uuid::Uuid::parse_str(teller_id).unwrap();
    let id = shift_id.clone();
    let b = "MAIN".to_string();
    let b_date = business_date.to_string();
    BranchTx::new(Transactor::new(desk.db.pool.clone()))
        .run(&BranchId::new("MAIN"), move |tx| {
            let id = id.clone();
            let b = b.clone();
            let b_date = b_date.clone();
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id, business_date) \
                     VALUES ($1, $2, 'OPEN', now(), 'test', '', $3, $4, $5::date)",
                )
                .bind(&id)
                .bind(&b)
                .bind(format!("REQ-{}", uuid::Uuid::new_v4()))
                .bind(teller_uuid)
                .bind(&b_date)
                .execute(&mut **tx)
                .await?;

                xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
                    .post_in(
                        tx,
                        &[xc_cash::domain::posting::PostingRequest {
                            from: xc_cash::domain::posting::CashPlace::Bank,
                            to: xc_cash::domain::posting::CashPlace::Drawer(id.clone()),
                            currency: xc_kernel::money::CurrencyCode::parse("THB").unwrap(),
                            quantity: xc_kernel::money::ForeignAmount::from_decimal(
                                rust_decimal::Decimal::from(100_000),
                            ),
                            source: xc_cash::domain::ledger::posting::SourceRef {
                                source_type: xc_cash::domain::ledger::posting::SourceType::Opening,
                                source_id: format!("r1-fund:{id}:THB"),
                                source_key: "fund".to_string(),
                            },
                            line: None,
                            reverses: None,
                        }],
                        None,
                    )
                    .await?;
                Ok::<(), pos_error::AppError>(())
            })
        })
        .await
        .unwrap();
    shift_id
}

/// Row V1: DEMO sale without `valueDate` (defaults 06/14/22) -> stored `value_date` = the business date of now under the defaults.
#[actix_web::test]
async fn cs23_v1_demo_sale_without_value_date() {
    let desk = Desk::start().await;
    let teller_id = seed_teller_and_passport(&desk).await;
    let customer_id = reserve_customer(&desk, "DEMO").await;
    let now = Utc::now();
    let q = desk.quote("MAIN", Side::Buy, "100", None, now).await;

    // Verify MAIN's shift plan yields the business date of now under the defaults (06/14/22)
    let expected = (now.with_timezone(&chrono_tz::Asia::Bangkok)
        - chrono::TimeDelta::hours(6))
    .date_naive();
    let mut tx = desk.db.pool.begin().await.unwrap();
    let plan = ShiftPlanService::new(
        Arc::new(PgShiftTemplateStore),
        BranchTx::new(Transactor::new(desk.db.pool.clone())),
    )
    .plan_in(&mut tx, &BranchId::new("MAIN"))
    .await
    .expect("MAIN shift plan");
    assert_eq!(plan.business_date_of(now), Some(expected));
    tx.rollback().await.unwrap();

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
        "notes": "",
        "countConfirmed": false,
    });

    let ans = post_transaction(&desk, &teller_id, &body).await;
    assert_eq!(ans.status, StatusCode::OK, "create: {}", ans.text);
    let id = ans.body["id"].as_str().expect("transaction id");

    let val_date = stored_value_date(&desk, id).await;
    assert_eq!(val_date, expected.to_string());
}

/// Row V2: DEMO sale with `valueDate 2026-10-06` -> stored 2026-10-06 (unchanged).
#[actix_web::test]
async fn cs23_v2_demo_sale_with_value_date() {
    let desk = Desk::start().await;
    let teller_id = seed_teller_and_passport(&desk).await;
    let customer_id = reserve_customer(&desk, "DEMO").await;
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

    let ans = post_transaction(&desk, &teller_id, &body).await;
    assert_eq!(ans.status, StatusCode::OK, "create: {}", ans.text);
    let id = ans.body["id"].as_str().expect("transaction id");

    let val_date = stored_value_date(&desk, id).await;
    assert_eq!(
        val_date, "2026-10-06",
        "DEMO sale with valueDate keeps provided date 2026-10-06"
    );
}

/// Row V3: `valueDate "06-10-2026"` -> `invalid valueDate` (unchanged).
#[actix_web::test]
async fn cs23_v3_malformed_value_date() {
    let desk = Desk::start().await;
    let teller_id = seed_teller_and_passport(&desk).await;
    let customer_id = reserve_customer(&desk, "DEMO").await;
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
        "valueDate": "06-10-2026",
        "notes": "",
        "countConfirmed": false,
    });

    let ans = post_transaction(&desk, &teller_id, &body).await;
    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "status: {}", ans.text);
    assert_eq!(ans.code(), "validationFailed");
    assert_eq!(ans.body["error"]["field"].as_str(), Some("valueDate"));
    assert_eq!(
        ans.body["error"]["message"].as_str(),
        Some("invalid valueDate")
    );
    assert_eq!(rows(&desk).await, 0, "nothing is written");
}

/// Row V4: LIVE sale without `valueDate` -> accepted; stored = its shift's business date.
#[actix_web::test]
async fn cs23_v4_live_sale_without_value_date() {
    let desk = Desk::start().await;
    let teller_id = seed_teller_and_passport(&desk).await;
    let customer_id = reserve_customer(&desk, "LIVE").await;
    let _shift = open_shift_for_teller(&desk, &teller_id, "2026-11-04").await;

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
        "notes": "",
        "countConfirmed": false,
    });

    let ans = post_transaction(&desk, &teller_id, &body).await;
    assert_eq!(ans.status, StatusCode::OK, "create: {}", ans.text);
    let id = ans.body["id"].as_str().expect("transaction id");

    let val_date = stored_value_date(&desk, id).await;
    assert_eq!(
        val_date, "2026-11-04",
        "LIVE sale without valueDate takes its shift's business date"
    );
}
