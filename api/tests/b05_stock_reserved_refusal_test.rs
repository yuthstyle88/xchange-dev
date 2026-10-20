//! Up-front tests for #3819 (B05): a completion refused **by a reservation** answers 409 `stockReserved` and opens
//! **no** `NEGATIVE_STOCK` approval request, because no approval can clear it (D-14: N-1 never reaches into another
//! quote's hold; D-16: a reservation never exceeds available). A plain below-zero shortfall keeps #3405's path
//! (`approvalRequired`), pinned in `s2_completion_negative_stock_test.rs`.
//! Ruling: claude/a90ab16c on #3819 (*"the two reservation refusals answer a new code `stockReserved`, which never
//! opens an approval"*). Written before the implementation by the B05 sub-controller claude/6530d490.
//! **Do not edit, delete or weaken**; add tests in a file of your own.
//!
//! The harness is `s2_completion_negative_stock_test.rs`'s, copied: everything after the setup goes over the wire,
//! through the real `LedgerCashPostings` and the registered exchange effects. MAIN's drawer opens with USD 1,000.00
//! (`tx5_common::Counter`); each test sets MAIN's USD position to 1,000 as well.
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
use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionStatus};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TELLER_AS: &str = "teller";

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let teller = req
            .headers()
            .get(AS)
            .is_some_and(|v| v.as_bytes() == TELLER_AS.as_bytes());
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: teller
                    .then(|| Actor::staff(TELLER, Role::Front, BranchId::new("MAIN")))
                    .flatten(),
            }
        })
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

/// The teller completes `t` over the wire with `key` as the `Idempotency-Key`.
async fn complete(c: &Counter, t: &ExchangeTransaction, key: &str) -> Answer {
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
        .uri(&format!("/api/transactions/{}/complete", t.id.0))
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, TELLER_AS))
        .insert_header(("idempotency-key", key))
        .set_payload(json!({}).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// Reserves `q`'s one line through B05's store at `now` (the binding quote's call, done here directly).
async fn hold(c: &Counter, q: &Quote, now: DateTime<Utc>) {
    let mut tx = c.desk.db.pool.begin().await.unwrap();
    PgStockReservations::new()
        .reserve_in(
            &mut tx,
            &ReserveLine {
                quote_line_id: q.lines[0].id,
                denomination_id: None,
            },
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("reserve_in: {e}"));
    tx.commit().await.unwrap();
}

/// MAIN's USD position at 1,000, the drawer's opening (set here so the figures do not depend on how the opening
/// was posted).
async fn usd_position_1000(c: &Counter) {
    c.desk
        .db
        .exec(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             SELECT organization_id, 'MAIN', 'USD', 1000, 32000 FROM branches WHERE id = 'MAIN' \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = 1000",
        )
        .await;
}

async fn negative_stock_requests(c: &Counter) -> i64 {
    c.desk
        .db
        .count("SELECT count(*) FROM approval_requests WHERE type = 'NEGATIVE_STOCK'")
        .await
}

async fn assert_not_completed(c: &Counter, t: &ExchangeTransaction) {
    let stored = c.desk.stored(t.id).await;
    assert_eq!(
        stored.status,
        TransactionStatus::Confirmed,
        "still CONFIRMED"
    );
    assert_eq!(stored.document_ref, None, "no number was issued");
}

/// SR-6's refusal: another quote holds 900 of MAIN's 1,000 USD, and a completion with no hold of its own sells 200.
#[actix_web::test]
async fn a_sale_into_another_quotes_hold_answers_stock_reserved_and_opens_no_request() {
    let c = Counter::start().await;
    usd_position_1000(&c).await;
    let now = whole_second_now();
    let other = c.desk.quote("MAIN", Side::Sell, "900", None, now).await;
    hold(&c, &other, now).await;
    let q = c.desk.quote("MAIN", Side::Sell, "200", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;

    let a = complete(&c, &t, &unique("complete")).await;

    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "stockReserved"),
        "{}",
        a.text
    );
    assert_eq!(
        negative_stock_requests(&c).await,
        0,
        "no approval can clear a hold, so none is opened"
    );
    assert_not_completed(&c, &t).await;
    c.finish().await;
}

/// SR-2's refusal: the exchange's own hold has lapsed and the stock it held is gone, so Consume cannot re-reserve.
#[actix_web::test]
async fn a_lapsed_own_hold_whose_stock_is_gone_answers_stock_reserved_and_opens_no_request() {
    let c = Counter::start().await;
    usd_position_1000(&c).await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "200", None, now).await;
    hold(&c, &q, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    // The hold lapsed, and another quote now holds the rest of the stock (1,000 − 900 = 100 < 200).
    c.desk
        .db
        .exec(&format!(
            "UPDATE stock_reservations SET expires_at = '{}' WHERE quote_line_id = '{}'",
            (now - Duration::seconds(1)).to_rfc3339(),
            q.lines[0].id
        ))
        .await;
    let other = c.desk.quote("MAIN", Side::Sell, "900", None, now).await;
    hold(&c, &other, now).await;

    let a = complete(&c, &t, &unique("complete")).await;

    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "stockReserved"),
        "{}",
        a.text
    );
    assert_eq!(negative_stock_requests(&c).await, 0);
    assert_not_completed(&c, &t).await;
    c.finish().await;
}

/// The same key again: still `stockReserved`, still no request (nothing was written by the first try).
#[actix_web::test]
async fn retrying_a_stock_reserved_refusal_opens_no_request_either() {
    let c = Counter::start().await;
    usd_position_1000(&c).await;
    let now = whole_second_now();
    let other = c.desk.quote("MAIN", Side::Sell, "900", None, now).await;
    hold(&c, &other, now).await;
    let q = c.desk.quote("MAIN", Side::Sell, "200", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    let key = unique("complete");

    let first = complete(&c, &t, &key).await;
    let again = complete(&c, &t, &unique("complete-again")).await;

    assert_eq!(first.code(), "stockReserved", "{}", first.text);
    assert_eq!(again.code(), "stockReserved", "{}", again.text);
    assert_eq!(negative_stock_requests(&c).await, 0);
    c.finish().await;
}

/// Control: with no hold anywhere, a sale beyond the drawer keeps #3405's path (`approvalRequired`, one request).
#[actix_web::test]
async fn with_no_hold_a_shortfall_still_asks_for_approval() {
    let c = Counter::start().await;
    usd_position_1000(&c).await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "2000", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;

    let a = complete(&c, &t, &unique("complete")).await;

    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "{}",
        a.text
    );
    assert_eq!(negative_stock_requests(&c).await, 1);
    c.finish().await;
}
