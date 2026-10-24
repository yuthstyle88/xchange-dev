//! `POST /api/quotes` checks the `customerId` it is given before it prices or stores anything.
//! Written before the implementation by the ticket author's helper (for claude/7ca80c). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! The defect at origin/main 3bac87e0a: `QuotesCommand` passes `customerId` straight into
//! `QuoteRequest.customer_id` (`api/src/presentation/http/v3/commands/quotes.rs:221-225`) and
//! `QuoteService` (`api/src/application/workflows/exchange/quote.rs`) never checks it. The only
//! guard is `quotes.customer_id`'s foreign key to `customers(id)`
//! (`api/migrations/20261014100000_quotes.sql`), which knows nothing of organisations. So:
//! - an unknown id reaches the insert and fails on the foreign key: `500 serverError`;
//! - another organisation's customer is quoted, and its segment, referral, consent and coupon rows
//!   feed the benefit engine (`quote.rs:324-391`).
//!
//! Behaviour this file pins:
//! | # | `customerId` | answer | stored |
//! |---|---|---|---|
//! | Q1 | no such customer | `400 validationFailed`, `field: "customerId"` | nothing |
//! | Q2 | a customer of another organisation | `403 outOfScope` | nothing |
//! | Q3a | a customer of the caller's organisation (no identity on file) | `200` | the quote, with `customer_id` |
//! | Q3b | a customer of the same organisation, registered at another of its branches | `200` | the quote |
//! | Q3c | none (walk-in) | `200` | the quote, `customer_id` NULL |
//!
//! Q2's code is the one the exchange already uses for another organisation's customer:
//! `CustomerService::exchange_customer_in` answers `outOfScope: customer … is not this branch's
//! organisation's` (`api/crates/xc-customer/src/application/customer/service.rs:240-244`), and the
//! v3 seam answers `outOfScope` as 403 (`v3_quotes_route_test.rs`, header). A quote must not take
//! the rest of `exchange_customer_in`, though: it refuses a customer with no ACTIVE PASSPORT
//! identity, and a quote is asked before the passport is read. Q3a seeds a customer with no
//! identity and must stay 200.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/quotes";

/// A second organisation and its one branch.
const ORG_2: &str = "00000000-0000-0000-0000-0000000000c2";
const ORG_2_BRANCH: &str = "XC-O2";

// ------------------------------------------------------------ the caller

/// MAIN's FRONT (TELLER) is signed in on every request: the caller `v3_quotes_route_test.rs` uses.
struct FrontOfMain;

impl SessionReader for FrontOfMain {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let roles = test_callers::v2_staff_roles("FRONT").expect("the FRONT role");
            test_callers::callers(
                None,
                Some(test_callers::staff(
                    "employee-FRONT",
                    roles,
                    BranchId::new("MAIN"),
                )),
            )
        })
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

    fn field(&self) -> &str {
        self.body["error"]["field"].as_str().unwrap_or("")
    }
}

/// `POST /api/quotes` with `body` and a fresh `Idempotency-Key`.
async fn ask(db: &TestDb, body: &Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(FrontOfMain);
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
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A one-line DESK quote of USD 10 bought, naming `customer` when given and no branch (the seam
/// runs it on the caller's MAIN).
fn body(customer: Option<&str>) -> Value {
    let mut b = json!({
        "channel": "DESK",
        "lines": [{ "currency": "USD", "shopSide": "BUY", "quantity": "10" }],
    });
    if let Some(customer) = customer {
        b["customerId"] = json!(customer);
    }
    b
}

/// The stored `customer_id` of the answered quote.
async fn stored_customer(db: &TestDb, a: &Answer) -> Option<String> {
    let id = a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the answer has an id: {}", a.text));
    db.text(&format!(
        "SELECT customer_id FROM quotes WHERE id = '{id}'::uuid"
    ))
    .await
}

// ------------------------------------------------------------ Q1

#[actix_web::test]
async fn an_unknown_customer_is_validation_failed_on_customer_id_and_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;

    let a = ask(&db, &body(Some("C-NOBODY"))).await;
    assert_eq!(
        (a.status, a.code(), a.field()),
        (StatusCode::BAD_REQUEST, "validationFailed", "customerId"),
        "no customer C-NOBODY: body {}",
        a.text
    );
    assert_eq!(db.quote_rows().await, (0, 0), "nothing stored");
    db.finish().await;
}

// ------------------------------------------------------------ Q2

#[actix_web::test]
async fn another_organisations_customer_is_out_of_scope_and_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    db.second_organisation(ORG_2, "ORG2", ORG_2_BRANCH).await;
    db.customer("C-OTHER", ORG_2_BRANCH).await;
    assert_eq!(
        db.text("SELECT organization_id::text FROM customers WHERE id = 'C-OTHER'")
            .await
            .as_deref(),
        Some(ORG_2),
        "the seeded customer is the second organisation's"
    );

    let a = ask(&db, &body(Some("C-OTHER"))).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "MAIN's FRONT names the second organisation's customer: body {}",
        a.text
    );
    assert_eq!(db.quote_rows().await, (0, 0), "nothing stored");
    assert_eq!(
        db.count("SELECT count(*) FROM benefit_reservations").await,
        0,
        "no benefit reserved for the other organisation's customer"
    );
    db.finish().await;
}

// ------------------------------------------------------------ Q3 (guards)

#[actix_web::test]
async fn a_customer_of_the_callers_organisation_is_quoted_and_stored() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    // no customer_identities row: a quote does not need the passport on file
    db.customer("C-1", "MAIN").await;

    let a = ask(&db, &body(Some("C-1"))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(
        stored_customer(&db, &a).await.as_deref(),
        Some("C-1"),
        "customer_id"
    );
    assert_eq!(db.quote_rows().await, (1, 1));
    db.finish().await;
}

#[actix_web::test]
async fn a_customer_registered_at_another_branch_of_the_same_organisation_is_quoted() {
    let db = TestDb::start().await;
    db.branch("XC-B01").await;
    db.publish("MAIN", &[USD]).await;
    // CI-4 #651: a customer is the organisation's, not the branch's
    db.customer("C-B01", "XC-B01").await;

    let a = ask(&db, &body(Some("C-B01"))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(
        stored_customer(&db, &a).await.as_deref(),
        Some("C-B01"),
        "customer_id"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_walk_in_quote_with_no_customer_is_still_answered() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;

    let a = ask(&db, &body(None)).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(stored_customer(&db, &a).await, None, "customer_id is NULL");
    assert_eq!(db.quote_rows().await, (1, 1));
    db.finish().await;
}
