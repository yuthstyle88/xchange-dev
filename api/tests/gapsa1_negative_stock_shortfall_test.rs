//! XC-BE-GAPS-A row 1 — the `NEGATIVE_STOCK` request a short completion opens carries the shortfall in
//! `proposed_payload` (design `docs/design/blueprint/ledger-inventory.md:418-425` §5.1: "requested by the caller that
//! got `stockInsufficient` … with the shortfall per currency in `proposed_payload`"). Until now the gate sent
//! `{"shortfall": []}` (`api/src/application/workflows/exchange/negative_stock_gate.rs`), which
//! `s2_completion_negative_stock_test.rs` left unpinned on purpose ("the shortfall figures in `proposed_payload`").
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 1, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Pinned here: the exchange of `s2_completion_negative_stock_test.rs` (a CONFIRMED LIVE cash SELL of USD 2,000 on
//! MAIN's open shift, whose drawer opens with USD 1,000.00 funded `BANK → DRAWER` through the ledger, so the
//! branch's USD position is 1,000 too) is completed over the wire; the one PENDING request's `proposed_payload` is
//! exactly the drawer short by 1,000 and the position short by 1,000, in the port's order (currency, then
//! `DRAWER` < `SAFE` < `POSITION`), `shortBy` the normalized decimal as a string. A second completion while the
//! request is pending opens nothing and leaves the payload as it was.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::ExchangeTransaction;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

/// The teller signs in for every request: FRONT of MAIN, id `TELLER`.
struct TellerSessions;

impl SessionReader for TellerSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let signed_in = req.headers().contains_key(AS);
        Box::pin(async move {
            if !signed_in {
                return test_callers::nobody();
            }
            test_callers::callers(
                None,
                Some(test_callers::staff(
                    TELLER,
                    test_callers::v2_staff_roles("FRONT").unwrap(),
                    BranchId::new("MAIN"),
                )),
            )
        })
    }
}

/// The teller completes `t` with `key`; answers the status, the error code and the raw body.
async fn complete(c: &Counter, t: &ExchangeTransaction, key: &str) -> (StatusCode, String, String) {
    let dir = c.scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        c.desk.db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(TellerSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(&format!("/api/transactions/{}/complete", t.id.0))
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, "teller"))
        .insert_header(("idempotency-key", key))
        .set_payload(json!({}).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let code = body["error"]["code"].as_str().unwrap_or("").to_owned();
    (status, code, text)
}

/// A CONFIRMED LIVE cash SELL of USD 2,000, beyond the drawer's USD 1,000.00.
async fn short_sell(c: &Counter) -> ExchangeTransaction {
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "2000", None, now).await;
    c.live_of(&q, PaymentMethod::Cash, true, now).await
}

/// The `NEGATIVE_STOCK` requests naming `t`, as `(status, proposed_payload)` in the order they were opened.
async fn requests(c: &Counter, t: &ExchangeTransaction) -> Vec<(String, Value)> {
    sqlx::query_as(
        "SELECT status, proposed_payload FROM approval_requests \
         WHERE type = 'NEGATIVE_STOCK' AND object_id = $1 ORDER BY requested_at, id",
    )
    .bind(t.id.0.to_string())
    .fetch_all(&c.desk.db.pool)
    .await
    .expect("read approval_requests")
}

fn expected_payload() -> Value {
    json!({
        "shortfall": [
            {"currency": "USD", "place": "DRAWER", "shortBy": "1000"},
            {"currency": "USD", "place": "POSITION", "shortBy": "1000"},
        ]
    })
}

#[actix_web::test]
async fn gapsa1_a_short_completion_opens_a_request_carrying_the_shortfall() {
    let c = Counter::start().await;
    let t = short_sell(&c).await;

    let (status, code, text) = complete(&c, &t, &unique("key")).await;
    assert_eq!(
        (status, code.as_str()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "§5.1: the desk is told to wait for approval: {text}"
    );
    let rows = requests(&c, &t).await;
    assert_eq!(rows.len(), 1, "one NEGATIVE_STOCK request: {rows:?}");
    assert_eq!(rows[0].0, "PENDING");
    assert_eq!(
        rows[0].1,
        expected_payload(),
        "§5.1: the drawer ends at 1,000 − 2,000 and the branch's USD position at 1,000 − 2,000"
    );
    c.finish().await;
}

#[actix_web::test]
async fn gapsa1_a_second_completion_while_pending_leaves_the_payload_as_it_was() {
    let c = Counter::start().await;
    let t = short_sell(&c).await;

    let (_, first, text) = complete(&c, &t, &unique("key")).await;
    assert_eq!(first, "approvalRequired", "{text}");
    let (_, again, text) = complete(&c, &t, &unique("key")).await;
    assert_eq!(again, "approvalRequired", "{text}");
    let rows = requests(&c, &t).await;
    assert_eq!(rows.len(), 1, "still one request: {rows:?}");
    assert_eq!(rows[0].1, expected_payload());
    c.finish().await;
}
