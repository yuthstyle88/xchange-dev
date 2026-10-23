//! Up-front tests for #4970 (P1): admin and desk text fields take a bounded length, and a segment's
//! `value_thb` follows the THB money rule.
//!
//! Written before the implementation by the ticket author's lane (claude/28619d18, for claude/7ca80c).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Today none of these fields has a length check (#4970 §1), so every `*_over_max_*` test answers 200
//! and fails, and every `*_at_max_*` test passes. Lengths are counted in chars after trim (§2.1).
//!
//! Behaviour rows (#4970 §3; one T2 and one T3 test per row of §1):
//! | # | Field (route) | Bound | at max (T3) | max+1 (T2) |
//! |---|---|---|---|---|
//! | 1 | certify `note` (`reconciliation_certify`) | 0..500 | 200 | 400 `note` |
//! | 2 | reject `reason` (`/api/approvals/{id}/decide`, REJECTED) | 1..500 | 200 | 400 `reason` |
//! | 2 | approve `reason` (same, APPROVED) | 0..500 | 200 | 400 `reason` |
//! | 3 | staff user `name` (`POST /api/users`) | 1..100 | 200 | 400 `name` |
//! | 4 | `employeeCode` (`POST /api/users/profile`) | 0..40 | 200 | 400 `employeeCode` |
//! | 5 | `department` (same) | 0..80 | 200 | 400 `department` |
//! | 6 | `position` (same) | 0..80 | 200 | 400 `position` |
//! | 7 | `tellCode` (same) | 0..40 | 200 | 400 `tellCode` |
//! | 8 | consent `body` (`POST /api/consent-texts`) | 1..20,000 | 200 | 400 `body` |
//! | 9 | segment `name` (`POST /api/segments`) | 1..80 | 200 | 400 `name` |
//! | 10 | segment `rule.value_thb` | MONEY ≤ 100,000,000 | `100000000.00` 200 | see T5 |
//! | 11 | `payment.reference` (`POST /api/transactions`) | 0..100 | 200 | 400 `payment.reference` |
//! | 12 | `payment.account` (same) | 0..100 | 200 | 400 `payment.account` |
//! | 13 | `notes` (same) | 0..500 | 200 | 400 `notes` |
//! | T4 | reject with a blank reason | | | 400 `reason` |
//! | T5 | segment `value_thb` `1.234` / `-1` / `100000000.01` | | | 400 `rule.value_thb` |

mod common;
mod identity_signin_common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use quote_common::TestDb;
use serde_json::{Value, json};
use tx4_common::{Desk, TELLER};
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
const MAIN: &str = "MAIN";
/// The owner's user id: a UUID, as `reconciliation_certify` parses the caller's id.
const OWNER_ID: &str = "00000000-0000-4000-8000-000000000001";
/// CI-4 (#651): the test PII key, as `tx4_routes_test.rs`.
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

// ------------------------------------------------------------ callers

/// Who is signed in, from the header `x-test-as`: `owner`, `teller` (FRONT of MAIN, tx4's
/// `TELLER`), or `staff:<V2ROLE>:<branch>` (id: a UUID per role).
struct HeaderCallers;

fn staff_id(role: &str) -> String {
    let n = test_callers::V2_STAFF
        .iter()
        .position(|(name, _)| *name == role)
        .unwrap_or_else(|| panic!("unknown staff role {role:?}"));
    format!("00000000-0000-4000-8000-{:012}", 100 + n)
}

fn callers_of(spec: &str) -> Callers {
    match spec {
        "" => test_callers::nobody(),
        "owner" => test_callers::callers(Some(test_callers::owner(OWNER_ID)), None),
        "teller" => test_callers::callers(
            None,
            Some(test_callers::staff(
                TELLER,
                &[Role::Teller],
                BranchId::new(MAIN),
            )),
        ),
        other => {
            let mut parts = other.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {other:?}");
            let role = parts.next().expect("a role");
            let branch = parts.next().expect("a branch");
            let roles = test_callers::v2_staff_roles(role).expect("a known v2 role");
            test_callers::callers(
                None,
                Some(test_callers::staff(
                    &staff_id(role),
                    roles,
                    BranchId::new(branch),
                )),
            )
        }
    }
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

async fn send(
    state: AppState,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> Answer {
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(body) = &body {
        req = req
            .insert_header(("content-type", "application/json"))
            .insert_header((
                "idempotency-key",
                xc_testdb::unique_schema_name("key").as_str(),
            ))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A fresh database whose owner (`OWNER_ID`) is a `users` row, since a UUID caller id is written as
/// `published_by` / `decided_by` and the like.
async fn start() -> TestDb {
    let db = TestDb::start().await;
    seed_owner(&db).await;
    db
}

async fn seed_owner(db: &TestDb) {
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         SELECT '{OWNER_ID}'::uuid, organization_id, 'owner', 'ACTIVE', 'test' FROM branches WHERE id = '{MAIN}'"
    ))
    .await;
}

fn state_of(db: &TestDb) -> AppState {
    AppState::new(Arc::new(common::settings(&[])), db.pool.clone())
}

async fn post(db: &TestDb, who: &str, uri: &str, body: Value) -> Answer {
    send(state_of(db), who, Method::POST, uri, Some(body)).await
}

fn accepted(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what} must be accepted: {}",
        a.text
    );
}

fn refused_on(a: &Answer, field: &str, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && a.body["error"]["code"] == "validationFailed"
            && a.body["error"]["field"] == field,
        "{what}: expected 400 validationFailed on {field:?}; got {} {}",
        a.status,
        a.text
    );
}

fn chars(n: usize) -> String {
    "a".repeat(n)
}

// ------------------------------------------------------------ 1. certify note (0..500)

const DAY: &str = "2026-10-05";

/// Certifies MAIN's sheet of `DAY` at its current hash with `note`, as MAIN's FINANCE.
async fn certify(note: &str) -> Answer {
    let db = start().await;
    let who = "staff:FINANCE:MAIN";
    let sheet = send(
        state_of(&db),
        who,
        Method::GET,
        &format!("/api/v3/views/finance_reconciliation?branch={MAIN}&date={DAY}"),
        None,
    )
    .await;
    assert_eq!(
        sheet.status,
        StatusCode::OK,
        "read the sheet: {}",
        sheet.text
    );
    let hash = sheet.body["data"]["figuresHash"]
        .as_str()
        .unwrap_or_else(|| panic!("figuresHash: {}", sheet.text))
        .to_owned();
    let a = post(
        &db,
        who,
        "/api/v3/commands/reconciliation_certify",
        json!({ "branch": MAIN, "date": DAY, "figuresHash": hash, "note": note }),
    )
    .await;
    db.finish().await;
    a
}

#[actix_web::test]
async fn certify_note_at_max_500_is_accepted() {
    accepted(&certify(&chars(500)).await, "a 500-char certify note");
}

#[actix_web::test]
async fn certify_note_over_max_501_is_refused_on_note() {
    refused_on(
        &certify(&chars(501)).await,
        "note",
        "a 501-char certify note",
    );
}

// ------------------------------------------------------------ 2. decide reason (REJECTED 1..500, APPROVED 0..500) and T4

/// A pending approval request: a RATES draft of MAIN submitted under no policy (out of bound).
async fn pending_request(db: &TestDb) -> String {
    let rates = "staff:RATES:MAIN";
    let draft = post(
        db,
        rates,
        "/api/rate-drafts",
        json!({ "lines": [{
            "currency": "USD", "buyMode": "MANUAL", "sellMode": "MANUAL",
            "manualBuy": "32.1", "manualSell": "32.45", "reason": "set by hand for the test",
        }] }),
    )
    .await;
    accepted(&draft, "create a draft");
    let id = draft.body["id"].as_str().expect("draft id").to_owned();
    let submitted = post(
        db,
        rates,
        &format!("/api/rate-drafts/{id}/submit"),
        json!({}),
    )
    .await;
    accepted(&submitted, "submit the draft");
    assert_eq!(
        submitted.body["status"], "PENDING_APPROVAL",
        "{}",
        submitted.text
    );
    submitted.body["requestId"]
        .as_str()
        .expect("a request id")
        .to_owned()
}

async fn decide(outcome: &str, reason: &str) -> Answer {
    let db = start().await;
    let request = pending_request(&db).await;
    let a = post(
        &db,
        "owner",
        &format!("/api/approvals/{request}/decide"),
        json!({ "branch": MAIN, "outcome": outcome, "reason": reason }),
    )
    .await;
    db.finish().await;
    a
}

#[actix_web::test]
async fn reject_reason_at_max_500_is_accepted() {
    accepted(
        &decide("REJECTED", &chars(500)).await,
        "a 500-char reject reason",
    );
}

#[actix_web::test]
async fn reject_reason_over_max_501_is_refused_on_reason() {
    refused_on(
        &decide("REJECTED", &chars(501)).await,
        "reason",
        "a 501-char reject reason",
    );
}

#[actix_web::test]
async fn approve_reason_at_max_500_is_accepted() {
    accepted(
        &decide("APPROVED", &chars(500)).await,
        "a 500-char approve reason",
    );
}

#[actix_web::test]
async fn approve_reason_over_max_501_is_refused_on_reason() {
    refused_on(
        &decide("APPROVED", &chars(501)).await,
        "reason",
        "a 501-char approve reason",
    );
}

#[actix_web::test]
async fn t4_reject_with_a_blank_reason_is_refused_on_reason() {
    refused_on(
        &decide("REJECTED", "   ").await,
        "reason",
        "a blank reject reason",
    );
}

// ------------------------------------------------------------ 3. staff user name (1..100)

async fn create_user(name: &str) -> Answer {
    let db = start().await;
    let identity = identity_signin_common::FakeIdentity::start();
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": uuid::Uuid::new_v4().to_string() }).to_string(),
    );
    let state = identity_signin_common::state_with(
        &db.pool,
        &identity_signin_common::identity_env(&identity.url()),
    );
    let a = send(
        state,
        "owner",
        Method::POST,
        "/api/users",
        Some(json!({
            "name": name,
            "phone": "+66812345674",
            "roles": [{ "role": "TELLER", "branch": MAIN }],
        })),
    )
    .await;
    db.finish().await;
    a
}

#[actix_web::test]
async fn user_name_at_max_100_is_accepted() {
    accepted(&create_user(&chars(100)).await, "a 100-char staff name");
}

#[actix_web::test]
async fn user_name_over_max_101_is_refused_on_name() {
    refused_on(
        &create_user(&chars(101)).await,
        "name",
        "a 101-char staff name",
    );
}

// ------------------------------------------------------------ 4-7. user profile (employeeCode 40, department 80, position 80, tellCode 40)

async fn save_profile(key: &str, value: &str) -> Answer {
    let db = start().await;
    let user = db
        .text(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, 'profile-target', 'ACTIVE', 'test' FROM branches WHERE id = 'MAIN' \
             RETURNING id::text",
        )
        .await
        .expect("a user of MAIN's organisation");
    let mut body = json!({
        "userId": user,
        "employeeCode": null,
        "department": null,
        "position": null,
        "tellCode": null,
    });
    body[key] = json!(value);
    let a = post(&db, "owner", "/api/users/profile", body).await;
    db.finish().await;
    a
}

#[actix_web::test]
async fn employee_code_at_max_40_is_accepted() {
    accepted(
        &save_profile("employeeCode", &chars(40)).await,
        "a 40-char employee code",
    );
}

#[actix_web::test]
async fn employee_code_over_max_41_is_refused_on_employee_code() {
    refused_on(
        &save_profile("employeeCode", &chars(41)).await,
        "employeeCode",
        "a 41-char employee code",
    );
}

#[actix_web::test]
async fn department_at_max_80_is_accepted() {
    accepted(
        &save_profile("department", &chars(80)).await,
        "an 80-char department",
    );
}

#[actix_web::test]
async fn department_over_max_81_is_refused_on_department() {
    refused_on(
        &save_profile("department", &chars(81)).await,
        "department",
        "an 81-char department",
    );
}

#[actix_web::test]
async fn position_at_max_80_is_accepted() {
    accepted(
        &save_profile("position", &chars(80)).await,
        "an 80-char position",
    );
}

#[actix_web::test]
async fn position_over_max_81_is_refused_on_position() {
    refused_on(
        &save_profile("position", &chars(81)).await,
        "position",
        "an 81-char position",
    );
}

#[actix_web::test]
async fn tell_code_at_max_40_is_accepted() {
    accepted(
        &save_profile("tellCode", &chars(40)).await,
        "a 40-char tell code",
    );
}

#[actix_web::test]
async fn tell_code_over_max_41_is_refused_on_tell_code() {
    refused_on(
        &save_profile("tellCode", &chars(41)).await,
        "tellCode",
        "a 41-char tell code",
    );
}

// ------------------------------------------------------------ 8. consent body (1..20,000)

async fn publish_consent(body: &str) -> Answer {
    let db = start().await;
    let a = post(
        &db,
        "owner",
        "/api/consent-texts",
        json!({ "purpose": "TERMS", "lang": "th", "body": body }),
    )
    .await;
    db.finish().await;
    a
}

#[actix_web::test]
async fn consent_body_at_max_20000_is_accepted() {
    accepted(
        &publish_consent(&chars(20_000)).await,
        "a 20,000-char consent body",
    );
}

#[actix_web::test]
async fn consent_body_over_max_20001_is_refused_on_body() {
    refused_on(
        &publish_consent(&chars(20_001)).await,
        "body",
        "a 20,001-char consent body",
    );
}

// ------------------------------------------------------------ 9-10. segment name (1..80) and value_thb (MONEY), T5

async fn define_segment(name: &str, rule: Value) -> Answer {
    let db = start().await;
    let a = post(
        &db,
        "owner",
        "/api/segments",
        json!({ "name": name, "rule": rule }),
    )
    .await;
    db.finish().await;
    a
}

#[actix_web::test]
async fn segment_name_at_max_80_is_accepted() {
    accepted(
        &define_segment(&chars(80), json!({ "value_thb": "5000.00" })).await,
        "an 80-char segment name",
    );
}

#[actix_web::test]
async fn segment_name_over_max_81_is_refused_on_name() {
    refused_on(
        &define_segment(&chars(81), json!({ "value_thb": "5000.00" })).await,
        "name",
        "an 81-char segment name",
    );
}

#[actix_web::test]
async fn segment_value_thb_at_max_100000000_is_accepted() {
    accepted(
        &define_segment("VIP", json!({ "value_thb": "100000000.00" })).await,
        "value_thb 100000000.00",
    );
}

#[actix_web::test]
async fn t5_segment_value_thb_with_three_decimals_is_refused() {
    refused_on(
        &define_segment("VIP", json!({ "value_thb": "1.234" })).await,
        "rule.value_thb",
        "value_thb 1.234",
    );
}

#[actix_web::test]
async fn t5_segment_value_thb_negative_is_refused() {
    refused_on(
        &define_segment("VIP", json!({ "value_thb": "-1" })).await,
        "rule.value_thb",
        "value_thb -1",
    );
}

#[actix_web::test]
async fn t5_segment_value_thb_over_100000000_is_refused() {
    refused_on(
        &define_segment("VIP", json!({ "value_thb": "100000000.01" })).await,
        "rule.value_thb",
        "value_thb 100000000.01",
    );
}

// ------------------------------------------------------------ 11-13. transaction payment reference / account (0..100), notes (0..500)

/// Creates a DRAFT at MAIN as the teller, with `reference`, `account` and `notes`.
async fn create_transaction(reference: &str, account: &str, notes: &str) -> Answer {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        let _ = xc_customer::install_pii_keys(keys);
    });
    let desk = Desk::start().await;
    seed_owner(&desk.db).await;
    let customer = CustomerService::new(BranchTx::new(Transactor::new(desk.db.pool.clone())))
        .reserve(
            &BranchId::new(MAIN),
            ReserveCustomer {
                name: "Somchai Jaidee".to_owned(),
                passport_no: format!("P{}", &uuid::Uuid::new_v4().simple().to_string()[..19])
                    .to_uppercase(),
                nationality: "THA".to_owned(),
                birth_date: String::new(),
                passport_expiry: String::new(),
                mode: "DEMO".to_owned(),
            },
            quote_common::whole_second_now(),
        )
        .await
        .unwrap_or_else(|e| panic!("reserve: {e}"))
        .customer_id;
    let quote = desk.quote(MAIN, Side::Buy, "100", None, Utc::now()).await;
    let a = post(
        &desk.db,
        "teller",
        "/api/transactions",
        json!({
            "quoteId": quote.id.to_string(),
            "customerId": customer,
            "passport": { "uploadKey": "PASSPORT-MAIN" },
            "payment": {
                "method": "Cash",
                "reference": reference,
                "account": account,
                "bankFeeThb": "0.00",
                "paidAt": Utc::now().to_rfc3339(),
            },
            "valueDate": "2026-10-06",
            "notes": notes,
            "countConfirmed": false,
        }),
    )
    .await;
    desk.db.finish().await;
    a
}

#[actix_web::test]
async fn payment_reference_at_max_100_is_accepted() {
    accepted(
        &create_transaction(&chars(100), "Cash drawer 1", "").await,
        "a 100-char payment reference",
    );
}

#[actix_web::test]
async fn payment_reference_over_max_101_is_refused_on_payment_reference() {
    refused_on(
        &create_transaction(&chars(101), "Cash drawer 1", "").await,
        "payment.reference",
        "a 101-char payment reference",
    );
}

#[actix_web::test]
async fn payment_account_at_max_100_is_accepted() {
    accepted(
        &create_transaction("R-1", &chars(100), "").await,
        "a 100-char payment account",
    );
}

#[actix_web::test]
async fn payment_account_over_max_101_is_refused_on_payment_account() {
    refused_on(
        &create_transaction("R-1", &chars(101), "").await,
        "payment.account",
        "a 101-char payment account",
    );
}

#[actix_web::test]
async fn transaction_notes_at_max_500_is_accepted() {
    accepted(
        &create_transaction("R-1", "Cash drawer 1", &chars(500)).await,
        "500-char transaction notes",
    );
}

#[actix_web::test]
async fn transaction_notes_over_max_501_is_refused_on_notes() {
    refused_on(
        &create_transaction("R-1", "Cash drawer 1", &chars(501)).await,
        "notes",
        "501-char transaction notes",
    );
}
