//! Backdated keying (`docs/design/backdated-keying.md` §4–§7, §10; behaviour table B1–B5, B8–B10, B13, B14; the
//! owner's rules R1–R4 and guard G of 2026-11-04): `POST /api/transactions/backdated` through the real app.
//! Written before the implementation by the sub-controller claude/6bb512ce. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! No shift is open anywhere: `tx4_common::Desk` opens none. MAIN's SAFE is funded through the ledger
//! (`lg4_fund`, BANK → SAFE). Each caller uploads its own passport scan (a `files` row it owns).
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
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::lg4_fund;
use xc_auth::domain::access::Role;
use xc_cash::domain::posting::CashPlace;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const OWNER: &str = "owner-bk";
const MANAGER: &str = "manager-bk";
const LIVE_CUSTOMER: &str = "CU-BK-LIVE";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// 2026-11-04 13:23 ICT: the first Telegram order of the owner's replay.
const PAID_AT: &str = "2026-11-04T13:23:00+07:00";
const VALUE_DATE: &str = "2026-11-04";

static PII: Once = Once::new();

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        let callers = match spec.as_str() {
            "owner" => test_callers::callers(Some(test_callers::owner(OWNER)), None),
            "manager" => test_callers::callers(
                None,
                Some(test_callers::staff(
                    MANAGER,
                    &[Role::BranchManager],
                    BranchId::new("MAIN"),
                )),
            ),
            "teller" => test_callers::callers(
                None,
                Some(test_callers::staff(
                    TELLER,
                    &[Role::Teller],
                    BranchId::new("MAIN"),
                )),
            ),
            other => panic!("unexpected caller {other:?}"),
        };
        Box::pin(async move { callers })
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

/// MAIN with no shift, a LIVE customer, a passport scan owned by each caller, and the SAFE holding THB `safe_thb`.
async fn start(safe_thb: &str) -> (Desk, std::path::PathBuf) {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        let _ = xc_customer::install_pii_keys(keys);
    });
    let desk = Desk::start().await;
    desk.db.customer(LIVE_CUSTOMER, "MAIN").await;
    for who in [OWNER, MANAGER, TELLER] {
        desk.db
            .exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('PASSPORT-BK-{who}', 'MAIN', '{who}', 'image/jpeg', 'sha-bk-{who}', now())"
            ))
            .await;
    }
    lg4_fund(
        &desk.db.pool,
        CashPlace::Bank,
        CashPlace::Safe(BranchId::new("MAIN")),
        "THB",
        safe_thb,
    )
    .await;
    let scratch = std::env::temp_dir().join(unique("xc-bk"));
    std::fs::create_dir_all(&scratch).unwrap();
    (desk, scratch)
}

/// B1's body: BUY 500 USD cash at 32.15, paid 2026-11-04 13:23 ICT.
fn body(who: &str) -> Value {
    json!({
        "customerId": LIVE_CUSTOMER,
        "side": "BUY",
        "currency": "USD",
        "quantity": "500",
        "rate": "32.15",
        "payment": {
            "method": "Cash",
            "reference": "",
            "account": "",
            "bankFeeThb": "0.00",
            "paidAt": PAID_AT,
        },
        "valueDate": VALUE_DATE,
        "passport": { "uploadKey": format!("PASSPORT-BK-{who}") },
        "notes": "Telegram order 1",
    })
}

async fn post(
    desk: &Desk,
    scratch: &std::path::Path,
    who: &str,
    key: &str,
    body: &Value,
) -> Answer {
    let dir = scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        desk.db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
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
        .insert_header((AS, who))
        .insert_header(("idempotency-key", key.to_owned()))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// MAIN's SAFE balance in `currency`, from the ledger.
async fn safe(desk: &Desk, currency: &str) -> Option<String> {
    desk.db
        .text(&format!(
            "SELECT coalesce(sum(e.quantity_signed), 0)::numeric(20,2)::text \
             FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
             WHERE a.branch_id = 'MAIN' AND a.kind = 'SAFE' AND a.currency = '{currency}'"
        ))
        .await
}

async fn finish(desk: Desk, scratch: std::path::PathBuf) {
    let _ = std::fs::remove_dir_all(&scratch);
    desk.db.finish().await;
}

/// B1: the Owner keys a past BUY with no shift; it is COMPLETED, backdated, numbered and dated on the deal's day,
/// and its cash moved in MAIN's SAFE.
#[actix_web::test]
async fn b1_owner_keys_a_past_buy_with_no_shift_into_the_safe() {
    let (desk, scratch) = start("100000.00").await;
    let a = post(&desk, &scratch, "owner", &unique("bk"), &body(OWNER)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["status"], "COMPLETED", "{}", a.text);
    assert_eq!(a.body["backdate"]["enteredBy"], OWNER, "{}", a.text);
    assert_eq!(a.body["lines"][0]["rateSnapshot"], "32.1500", "{}", a.text);
    assert_eq!(a.body["lines"][0]["settleThb"], "16075.00", "{}", a.text);
    let id = a.body["id"].as_str().expect("an id").to_owned();

    let row = desk
        .db
        .text(&format!(
            "SELECT concat_ws('|', backdated, shift_id IS NULL, quote_id IS NULL, recorded_by, value_date, \
                 paid_at = '{PAID_AT}'::timestamptz, completed_at = '{PAID_AT}'::timestamptz, \
                 document_ref LIKE 'XC-20261104-%', recorded_at > now() - interval '10 minutes') \
             FROM transactions t JOIN transaction_payments p ON p.transaction_id = t.id WHERE t.id = '{id}'"
        ))
        .await;
    assert_eq!(
        row.as_deref(),
        Some(format!("true|true|true|{OWNER}|2026-11-04|true|true|true|true").as_str()),
        "backdated, no shift, no quote, keyed by the owner now, dated and numbered on the deal's day"
    );
    assert_eq!(safe(&desk, "USD").await.as_deref(), Some("500.00"));
    assert_eq!(
        safe(&desk, "THB").await.as_deref(),
        Some("83925.00"),
        "100000.00 - 16075.00"
    );
    finish(desk, scratch).await;
}

/// B2: a MAIN Branch Manager may key it too.
#[actix_web::test]
async fn b2_a_branch_manager_keys_it_too() {
    let (desk, scratch) = start("100000.00").await;
    let a = post(&desk, &scratch, "manager", &unique("bk"), &body(MANAGER)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["backdate"]["enteredBy"], MANAGER, "{}", a.text);
    finish(desk, scratch).await;
}

/// B3 (R1): a teller is refused at the seam; nothing is written.
#[actix_web::test]
async fn b3_a_teller_is_refused() {
    let (desk, scratch) = start("100000.00").await;
    let a = post(&desk, &scratch, "teller", &unique("bk"), &body(TELLER)).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(a.code(), "roleNotAllowedHere", "{}", a.text);
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 0);
    finish(desk, scratch).await;
}

/// B4 (G): a certified business date is closed: 409 dayClosed, no row, no posting.
#[actix_web::test]
async fn b4_a_closed_day_is_refused() {
    let (desk, scratch) = start("100000.00").await;
    desk.db
        .exec(&format!(
            "INSERT INTO reconciliation_certifications (id, organization_id, branch_id, business_date, \
                 figures_hash, figures, certified_by, certified_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{VALUE_DATE}', 'h', '{{}}', gen_random_uuid(), now())"
        ))
        .await;
    let a = post(&desk, &scratch, "owner", &unique("bk"), &body(OWNER)).await;
    assert_eq!(a.status, StatusCode::CONFLICT, "{}", a.text);
    assert_eq!(a.code(), "dayClosed", "{}", a.text);
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 0);
    assert_eq!(safe(&desk, "USD").await.as_deref(), Some("0.00"));
    finish(desk, scratch).await;
}

/// B5: a deal time in the future is refused, naming `payment.paidAt`.
#[actix_web::test]
async fn b5_paid_at_in_the_future_is_refused() {
    let (desk, scratch) = start("100000.00").await;
    let mut b = body(OWNER);
    let later = chrono::Utc::now() + chrono::Duration::hours(2);
    b["payment"]["paidAt"] = json!(later.to_rfc3339());
    b["valueDate"] = json!(
        later
            .with_timezone(&chrono_tz::Asia::Bangkok)
            .date_naive()
            .to_string()
    );
    let a = post(&desk, &scratch, "owner", &unique("bk"), &b).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.code(), "validationFailed", "{}", a.text);
    assert_eq!(a.field(), "payment.paidAt", "{}", a.text);
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 0);
    finish(desk, scratch).await;
}

/// B6: the business date must be the deal's Bangkok date.
#[actix_web::test]
async fn b6_value_date_not_the_deal_date_is_refused() {
    let (desk, scratch) = start("100000.00").await;
    let mut b = body(OWNER);
    b["valueDate"] = json!("2026-11-03");
    let a = post(&desk, &scratch, "owner", &unique("bk"), &b).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.field(), "valueDate", "{}", a.text);
    finish(desk, scratch).await;
}

/// B7: a rate with 9 decimal places is refused, naming `rate`.
#[actix_web::test]
async fn b7_a_rate_with_nine_places_is_refused() {
    let (desk, scratch) = start("100000.00").await;
    let mut b = body(OWNER);
    b["rate"] = json!("32.150000001");
    let a = post(&desk, &scratch, "owner", &unique("bk"), &b).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.field(), "rate", "{}", a.text);
    finish(desk, scratch).await;
}

/// B8: a THB figure that differs from the computed settle is refused and names the computed one.
#[actix_web::test]
async fn b8_expected_settle_mismatch_is_refused() {
    let (desk, scratch) = start("100000.00").await;
    let mut b = body(OWNER);
    b["expectedSettleThb"] = json!("16075.01");
    let a = post(&desk, &scratch, "owner", &unique("bk"), &b).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.field(), "expectedSettleThb", "{}", a.text);
    assert!(a.message().contains("16075.00"), "{}", a.text);
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 0);
    finish(desk, scratch).await;
}

/// B9 (R3): the SAFE never goes below zero: THB 100.00 cannot pay 16075.00; nothing is written.
#[actix_web::test]
async fn b9_a_short_safe_is_refused() {
    let (desk, scratch) = start("100.00").await;
    let a = post(&desk, &scratch, "owner", &unique("bk"), &body(OWNER)).await;
    assert_eq!(a.status, StatusCode::CONFLICT, "{}", a.text);
    assert_eq!(a.code(), "stockInsufficient", "{}", a.text);
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 0);
    assert_eq!(safe(&desk, "THB").await.as_deref(), Some("100.00"));
    finish(desk, scratch).await;
}

/// B10: backdated keying is LIVE only: a DEMO customer is refused, naming `customerId`.
#[actix_web::test]
async fn b10_a_demo_customer_is_refused() {
    let (desk, scratch) = start("100000.00").await;
    let mut b = body(OWNER);
    b["customerId"] = json!(CUSTOMER);
    let a = post(&desk, &scratch, "owner", &unique("bk"), &b).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.field(), "customerId", "{}", a.text);
    finish(desk, scratch).await;
}

/// B13 (§10): the database itself refuses a LIVE row with no shift that is not backdated.
#[actix_web::test]
async fn b13_the_schema_refuses_a_live_row_without_a_shift_unless_backdated() {
    let (desk, scratch) = start("100000.00").await;
    let a = post(&desk, &scratch, "owner", &unique("bk"), &body(OWNER)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let id = a.body["id"].as_str().expect("an id").to_owned();
    assert_eq!(
        desk.db
            .outcome(&format!(
                "UPDATE transactions SET backdated = false WHERE id = '{id}'"
            ))
            .await,
        "23514",
        "check_violation of transactions_shift_by_mode_check"
    );
    finish(desk, scratch).await;
}

/// B14: the same Idempotency-Key and body answers the first sale again; one row.
#[actix_web::test]
async fn b14_a_repeat_with_the_same_key_replays() {
    let (desk, scratch) = start("100000.00").await;
    let key = unique("bk");
    let first = post(&desk, &scratch, "owner", &key, &body(OWNER)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let again = post(&desk, &scratch, "owner", &key, &body(OWNER)).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(first.body["id"], again.body["id"]);
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 1);
    finish(desk, scratch).await;
}
