//! HTTP test for ticket CS-8c (#6292): `POST /api/transactions` for a LIVE sale on a shift
//! whose business date is certified answers 409 `dayClosed`.

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

#[actix_web::test]
async fn cs8c_live_sale_on_certified_shift_day_answers_409_day_closed() {
    let desk = Desk::start().await;
    let teller_id = seed_teller_and_passport(&desk).await;
    let customer_id = reserve_customer(&desk, "LIVE").await;
    let date = "2026-11-04";
    let _shift = open_shift_for_teller(&desk, &teller_id, date).await;
    desk.db
        .exec(&format!(
            "INSERT INTO reconciliation_certifications (id, organization_id, branch_id, business_date, \
                 figures_hash, figures, certified_by, certified_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{date}', 'h', '{{}}', gen_random_uuid(), now())"
        ))
        .await;

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
    assert_eq!(ans.status, StatusCode::CONFLICT, "status: {}", ans.text);
    assert_eq!(ans.code(), "dayClosed");
    assert_eq!(
        desk.db.count("SELECT count(*) FROM transactions").await,
        0,
        "nothing is written"
    );
    desk.db.finish().await;
}
