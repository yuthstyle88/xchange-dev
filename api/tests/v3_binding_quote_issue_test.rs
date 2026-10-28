//! Up-front tests for BQ-5 (design `docs/design/blueprint/binding-quote.md` D-7, D-10, D-12, D-13, D-14, D-15;
//! §4.2 Q1, Q6, Q13, Q14, Q15). Written by the sub-controller claude/644b3958 on `feat/binding-quote` before the
//! implementation. **Do not edit, delete or weaken.**
//!
//! `POST /api/quotes/binding` (`commands/binding_quote_issue.rs`, skeleton) through the CM-1 command seam. Its
//! policy row (`xc-auth/.../areas/binding_quote.rs`) needs `Permission::BindingQuoteIssue`, which only the built-in
//! TELLER and BRANCH_MANAGER seeds hold after BQ-7; `run` builds `IssueBindingQuote`'s request from the body
//! (`quoteCreate`'s, plus `approvalRequestId` / `replacesQuoteId`; `channel` defaults to `DESK`) and answers:
//! - issued ⇒ 200 `{ quote, reservedLines }`: `quote` is `QuoteDto` (`type` `BINDING`, `expiresAt` the lock's end),
//!   `reservedLines` `[{ quoteLineId, reservedQuantity }]`, one per quote line (D-12);
//! - binding quotes off (no lifetime) ⇒ 400 `validationFailed` (D-10);
//! - a channel other than DESK ⇒ 400 `validationFailed`, field `channel` (D-14);
//! - over the branch limit ⇒ 409 `approvalRequired`, message `approvalRequired: <request id>` (D-15);
//! - a replay of the same `Idempotency-Key` and body answers the first answer and issues nothing more (D-7).
//!
//! Fixtures: MAIN's USD at 32.10 / 32.45, a customer of the organisation VERIFIED until 30 days from now (D-17),
//! MAIN holding 300 USD, `binding_quote_ttl_seconds = 600` (needs BQ-0's column). Callers come from the header
//! `x-test-as: <ROLE>:<branch>`, built with `test_callers::staff`, so the permission answer is the seed's.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/quotes/binding";
const TELLER: &str = "TELLER:MAIN";
const CUSTOMER_ID: &str = "CU-BQ5";
const TTL: i64 = 600;

/// The codes a policy refusal answers with (`V3Error::from(PolicyError)`).
const POLICY_CODES: [&str; 6] = [
    "signIn",
    "ownerOnly",
    "employeeAuthRequired",
    "invalidRequest",
    "roleNotAllowed",
    "roleNotAllowedHere",
];

// ------------------------------------------------------------ callers

struct HeaderCallers;

fn role_of(name: &str) -> Role {
    match name {
        "EXECUTIVE" => Role::Executive,
        "ADMIN" => Role::Admin,
        "RATE_MANAGER" => Role::RateManager,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "FINANCE" => Role::Finance,
        "MARKETING" => Role::Marketing,
        "CUSTOMER" => Role::Customer,
        "VERIFICATION_REVIEWER" => Role::VerificationReviewer,
        other => panic!("unknown role {other:?}"),
    }
}

/// `<ROLE>:<branch>` ⇒ a staff member holding that one role at that branch.
fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::nobody();
    }
    let (role, branch) = spec.split_once(':').expect("<ROLE>:<branch>");
    let actor = test_callers::staff(
        &format!("employee-{role}"),
        &[role_of(role)],
        BranchId::new(branch),
    );
    test_callers::callers(None, Some(actor))
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

async fn post(db: &TestDb, who: &str, body: &Value, key: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, who))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn ask(db: &TestDb, who: &str, body: &Value) -> Answer {
    post(db, who, body, &unique("key")).await
}

/// SELL `quantity` USD to the verified customer, on `channel`.
fn body_on(channel: &str, quantity: &str) -> Value {
    json!({
        "customerId": CUSTOMER_ID,
        "channel": channel,
        "lines": [{ "currency": "USD", "shopSide": "SELL", "quantity": quantity }],
    })
}

fn body(quantity: &str) -> Value {
    body_on("DESK", quantity)
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

// ------------------------------------------------------------ fixtures

/// MAIN priced, the customer verified, MAIN holding 300 USD; binding quotes on (600 s) when `ttl` is set.
async fn counter(ttl: Option<i64>) -> TestDb {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    db.customer(CUSTOMER_ID, "MAIN").await;
    let reviewer = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ('{reviewer}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'test')",
        unique("reviewer")
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         version, submitted_at, decided_at, expires_at, reviewed_by, created_at) \
         VALUES ('{}', '{ORG_1}'::uuid, '{CUSTOMER_ID}', 'STAFF', 'VERIFIED', 1, 1, now(), now(), \
         now() + interval '30 days', '{reviewer}', now())",
        Uuid::new_v4()
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
         VALUES ('{ORG_1}', 'MAIN', 'USD', 300, 9600) \
         ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = EXCLUDED.quantity"
    ))
    .await;
    if let Some(ttl) = ttl {
        db.exec(&format!(
            "UPDATE exchange_settings SET binding_quote_ttl_seconds = {ttl} WHERE organization_id = '{ORG_1}'"
        ))
        .await;
    }
    db
}

async fn reservations(db: &TestDb) -> i64 {
    db.count("SELECT count(*) FROM stock_reservations").await
}

/// Refused with `code` and `status` (and `field`, when given), and nothing was stored.
async fn refused(
    db: &TestDb,
    a: &Answer,
    status: StatusCode,
    code: &str,
    field: Option<&str>,
    what: &str,
) {
    assert_eq!((a.status, a.code()), (status, code), "{what}: {}", a.text);
    if let Some(field) = field {
        assert_eq!(
            a.body["error"]["field"],
            json!(field),
            "{what}: field: {}",
            a.text
        );
    }
    assert_eq!(db.quote_rows().await, (0, 0), "{what}: no quote");
    assert_eq!(reservations(db).await, 0, "{what}: no reservation");
}

// ------------------------------------------------------------ Q13, Q14

#[actix_web::test]
async fn q13_the_answer_says_what_is_locked_and_until_when() {
    let db = counter(Some(TTL)).await;
    let before = Utc::now();
    let a = ask(&db, TELLER, &body("100")).await;
    let after = Utc::now();
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);

    let q = &a.body["quote"];
    assert_eq!(q["type"], json!("BINDING"), "quote.type: {}", a.text);
    let id = q["id"]
        .as_str()
        .unwrap_or_else(|| panic!("quote.id: {}", a.text));
    let (ty, expires_at): (String, DateTime<Utc>) =
        sqlx::query_as("SELECT type, expires_at FROM quotes WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .expect("the answered quote is stored");
    assert_eq!(ty, "BINDING", "the stored type");
    let answered = DateTime::parse_from_rfc3339(
        q["expiresAt"]
            .as_str()
            .unwrap_or_else(|| panic!("quote.expiresAt: {}", a.text)),
    )
    .expect("expiresAt is RFC 3339")
    .with_timezone(&Utc);
    assert!(
        (answered - expires_at).num_milliseconds().abs() < 1000,
        "expiresAt {answered} is the stored {expires_at}"
    );
    assert!(
        answered >= before + Duration::seconds(TTL) - Duration::seconds(1)
            && answered <= after + Duration::seconds(TTL) + Duration::seconds(1),
        "expiresAt {answered} is now + the binding lifetime ({TTL} s)"
    );

    let lines = a.body["reservedLines"]
        .as_array()
        .unwrap_or_else(|| panic!("reservedLines: {}", a.text));
    assert_eq!(lines.len(), 1, "one per quote line");
    let line_id: String =
        sqlx::query_scalar("SELECT id::text FROM quote_lines WHERE quote_id = $1::uuid")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .expect("the quote line");
    assert_eq!(lines[0]["quoteLineId"], json!(line_id), "quoteLineId");
    assert_eq!(
        d(lines[0]["reservedQuantity"]
            .as_str()
            .unwrap_or_else(|| panic!("reservedQuantity is a string: {}", a.text))),
        d("100"),
        "reservedQuantity"
    );
    let (status, quantity, reserved_until): (String, Decimal, DateTime<Utc>) = sqlx::query_as(
        "SELECT status, quantity, expires_at FROM stock_reservations WHERE quote_line_id = $1::uuid",
    )
    .bind(&line_id)
    .fetch_one(&db.pool)
    .await
    .expect("one stock reservation for the line");
    assert_eq!(
        (status.as_str(), quantity),
        ("RESERVED", d("100")),
        "the reservation"
    );
    assert_eq!(
        reserved_until, expires_at,
        "the reservation ends with the quote"
    );
    db.finish().await;
}

#[actix_web::test]
async fn q14_a_teller_and_a_branch_manager_may_issue() {
    let db = counter(Some(TTL)).await;
    for who in [TELLER, "BRANCH_MANAGER:MAIN"] {
        let a = ask(&db, who, &body("10")).await;
        assert_eq!(a.status, StatusCode::OK, "{who}: {}", a.text);
        assert_eq!(a.body["quote"]["type"], json!("BINDING"), "{who}");
    }
    assert_eq!(db.quote_rows().await, (2, 2), "two quotes");
    assert_eq!(reservations(&db).await, 2, "two reservations");
    db.finish().await;
}

// ------------------------------------------------------------ Q15

#[actix_web::test]
async fn q15_every_other_role_is_refused_by_the_permission_answer() {
    let db = counter(Some(TTL)).await;
    for role in [
        "EXECUTIVE",
        "ADMIN",
        "RATE_MANAGER",
        "FINANCE",
        "MARKETING",
        "CUSTOMER",
    ] {
        let who = format!("{role}:MAIN");
        let a = ask(&db, &who, &body("10")).await;
        assert!(
            a.status.is_client_error(),
            "{who}: refused, got {}: {}",
            a.status,
            a.text
        );
        assert!(
            POLICY_CODES.contains(&a.code()),
            "{who}: a policy refusal code, got {:?}: {}",
            a.code(),
            a.text
        );
        assert_eq!(db.quote_rows().await, (0, 0), "{who}: no quote");
        assert_eq!(reservations(&db).await, 0, "{who}: no reservation");
    }
    db.finish().await;
}

// ------------------------------------------------------------ Q1, D-14, D-15

#[actix_web::test]
async fn q1_with_no_lifetime_binding_quotes_are_off() {
    let db = counter(None).await;
    let a = ask(&db, TELLER, &body("10")).await;
    refused(
        &db,
        &a,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        None,
        "off",
    )
    .await;
    db.finish().await;
}

#[actix_web::test]
async fn a_channel_other_than_desk_is_refused_on_channel() {
    let db = counter(Some(TTL)).await;
    for channel in ["APP", "KIOSK", "BOOKING"] {
        let a = ask(&db, TELLER, &body_on(channel, "10")).await;
        refused(
            &db,
            &a,
            StatusCode::BAD_REQUEST,
            "validationFailed",
            Some("channel"),
            channel,
        )
        .await;
    }
    db.finish().await;
}

#[actix_web::test]
async fn over_the_branch_limit_answers_approval_required_with_the_request_id() {
    let db = counter(Some(TTL)).await;
    db.exec(&format!(
        "INSERT INTO branch_exchange_limits (organization_id, branch_id, approval_limit_thb, updated_by, updated_at) \
         VALUES ('{ORG_1}', 'MAIN', 100, 'admin', now())"
    ))
    .await;
    let a = ask(&db, TELLER, &body("100")).await;
    refused(
        &db,
        &a,
        StatusCode::CONFLICT,
        "approvalRequired",
        None,
        "over the limit",
    )
    .await;
    let message = a.body["error"]["message"].as_str().unwrap_or("");
    let request_id = message
        .strip_prefix("approvalRequired: ")
        .unwrap_or_else(|| panic!("message names the request: {}", a.text));
    let row = db
        .text(&format!(
            "SELECT type || '|' || status FROM approval_requests WHERE id::text = '{request_id}'"
        ))
        .await;
    assert_eq!(
        row.as_deref(),
        Some("BINDING_QUOTE|PENDING"),
        "the named request"
    );
    db.finish().await;
}

// ------------------------------------------------------------ Q6

#[actix_web::test]
async fn q6_the_same_key_and_body_twice_issue_one_quote() {
    let db = counter(Some(TTL)).await;
    let key = unique("key");
    let first = post(&db, TELLER, &body("100"), &key).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let second = post(&db, TELLER, &body("100"), &key).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text);
    assert_eq!(
        second.body, first.body,
        "the replay answers the first answer"
    );
    assert_eq!(db.quote_rows().await, (1, 1), "one quote");
    assert_eq!(reservations(&db).await, 1, "one reservation");
    db.finish().await;
}
