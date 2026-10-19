//! R-quotes (#2054 row 2, design `docs/design/blueprint/bounded-inputs.md` §4 R-*) up-front tests — written before
//! the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller claude/56a74fcd. Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! `quoteCreate` (`POST /api/quotes`) parses `lines[].quantity` with `ForeignAmount::parse` and the line currency's
//! units: positive, at most the currency's minor-unit decimals, at most `Money::MAX_INPUT_UNITS` (10^8, inclusive).
//! A refusal is **400 `validationFailed` on `quantity`** and nothing is stored. Both sides of each bound are pinned.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/quotes";

/// Every request is a FRONT employee of MAIN.
struct Front;

impl SessionReader for Front {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            test_callers::callers(
                None,
                Some(test_callers::staff(
                    "employee-FRONT",
                    &[Role::Teller],
                    BranchId::new("MAIN"),
                )),
            )
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn ask(db: &TestDb, currency: &str, quantity: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Front);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let body = json!({
        "channel": "DESK",
        "lines": [{ "currency": currency, "shopSide": "BUY", "quantity": quantity }]
    });
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

async fn accepted(db: &TestDb, currency: &str, quantity: &str) {
    let before = db.quote_rows().await;
    let a = ask(db, currency, quantity).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{currency} {quantity:?} must be quoted: {}",
        a.text
    );
    let after = db.quote_rows().await;
    assert_eq!(
        (after.0 - before.0, after.1 - before.1),
        (1, 1),
        "{quantity:?}: one quote stored"
    );
}

async fn refused(db: &TestDb, currency: &str, quantity: &str) {
    let before = db.quote_rows().await;
    let a = ask(db, currency, quantity).await;
    assert_eq!(
        (a.status, a.body["error"]["code"].as_str().unwrap_or("")),
        (StatusCode::BAD_REQUEST, "validationFailed"),
        "{currency} {quantity:?}: body {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"].as_str(),
        Some("quantity"),
        "{currency} {quantity:?}: field, body {}",
        a.text
    );
    assert_eq!(
        db.quote_rows().await,
        before,
        "{currency} {quantity:?}: nothing stored"
    );
}

/// Row 1. The cap is inclusive: 10^8 units is quoted, one hundredth above is refused.
#[actix_web::test]
async fn quantity_cap_is_inclusive_at_one_hundred_million() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    accepted(&db, "USD", "100000000").await;
    accepted(&db, "USD", "100000000.00").await;
    refused(&db, "USD", "100000000.01").await;
    refused(&db, "USD", "100000001").await;
    db.finish().await;
}

/// Row 2. A quantity far past the column (`quote_lines.quantity numeric(20,4)`) is 400, never 500.
#[actix_web::test]
async fn a_quantity_past_the_column_is_400_not_500() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    refused(&db, "USD", "10000000000000000").await;
    refused(&db, "USD", &"9".repeat(40)).await;
    db.finish().await;
}

/// Row 3. At most the currency's minor-unit decimals: USD has 2, JPY has 0. Extra zeros are fine.
#[actix_web::test]
async fn quantity_decimals_follow_the_currencys_minor_unit() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD, JPY]).await;
    accepted(&db, "USD", "1.23").await;
    accepted(&db, "USD", "1.230").await;
    refused(&db, "USD", "1.234").await;
    accepted(&db, "JPY", "1000").await;
    accepted(&db, "JPY", "1000.00").await;
    refused(&db, "JPY", "1000.5").await;
    db.finish().await;
}

/// Row 4. Zero, negatives and anything that is not a plain decimal are refused on `quantity`.
#[actix_web::test]
async fn zero_negative_and_non_plain_quantities_are_refused() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    for q in [
        "0", "0.00", "-1", "+1", " 1", "1 ", "1e3", ".5", "5.", "abc", "",
    ] {
        refused(&db, "USD", q).await;
    }
    db.finish().await;
}
