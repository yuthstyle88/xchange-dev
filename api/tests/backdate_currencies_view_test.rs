//! F46a up-front tests (`docs/design/backdated-keying.md` §15.2, §15.3 row F46-1): `GET /api/v3/views/backdate_currencies`
//! lists every currency of the `currencies` table except THB, with the unit its rate is quoted per, in one normalised
//! form. It is the table `BackdateSale::run_in` prices from, so the backdated form cannot show a unit the server does
//! not use. Written before the implementation by the sub-controller claude/3b81c9d5. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! The units come from the migrations: `20261010100000_rate_units.sql` sets JPY 100 and KRW 1000, every other code 1,
//! and minor unit 0 for JPY and KRW.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde_json::Value;
use tx4_common::*;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const OWNER: &str = "owner-f46";

struct OwnerSession;

impl SessionReader for OwnerSession {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let callers = test_callers::callers(Some(test_callers::owner(OWNER)), None);
        Box::pin(async move { callers })
    }
}

/// The view's `data`, read as the Owner on MAIN.
async fn read(desk: &Desk) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(OwnerSession);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/api/v3/views/backdate_currencies?branch=MAIN")
        .insert_header((header::HOST, "xc.test"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn row<'a>(currencies: &'a [Value], code: &str) -> &'a Value {
    currencies
        .iter()
        .find(|r| r["code"] == code)
        .unwrap_or_else(|| panic!("{code} is listed"))
}

/// F46-1: every non-THB currency of the table, ascending, with its normalised unit and minor unit.
#[actix_web::test]
async fn f46_1_lists_every_non_thb_currency_with_its_unit() {
    let desk = Desk::start().await;
    let (status, body) = read(&desk).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let currencies = body["data"]["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("data.currencies is an array: {body}"))
        .clone();

    let in_table = desk
        .db
        .count("SELECT count(*) FROM currencies WHERE trim(code) <> 'THB'")
        .await;
    assert_eq!(
        currencies.len() as i64,
        in_table,
        "one row per non-THB currency"
    );
    assert!(
        currencies.iter().all(|r| r["code"] != "THB"),
        "THB is not a foreign currency"
    );
    let codes: Vec<&str> = currencies
        .iter()
        .map(|r| r["code"].as_str().expect("a code"))
        .collect();
    let mut sorted = codes.clone();
    sorted.sort_unstable();
    assert_eq!(codes, sorted, "ascending by code");

    assert_eq!(row(&currencies, "USD")["unitSize"], "1");
    assert_eq!(row(&currencies, "CAD")["unitSize"], "1");
    assert_eq!(row(&currencies, "JPY")["unitSize"], "100");
    assert_eq!(row(&currencies, "KRW")["unitSize"], "1000");
    assert_eq!(row(&currencies, "JPY")["minorUnit"], 0);
    assert_eq!(row(&currencies, "USD")["minorUnit"], 2);
    assert!(
        currencies
            .iter()
            .all(|r| !r["unitSize"].as_str().unwrap_or("").contains('.')),
        "units are normalised, never \"1.0000\": {currencies:?}"
    );
    desk.db.finish().await;
}
