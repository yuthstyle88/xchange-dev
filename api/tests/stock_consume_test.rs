//! Up-front tests for SR-2 (B05: consume at completion; design `docs/design/blueprint/stock-reservation.md` D-5,
//! §4.2 S8, S10, S12). Written by the B05 sub-controller claude/6530d490 on `feat/b05-stock` before the
//! implementation. **Do not edit, delete or weaken.**
//!
//! `CompleteExchange` runs the new effect phase `Consume` (the stock effect, `bootstrap/exchange_effects/stock.rs`)
//! in the caller's transaction **before** `post_in`. The service is built with the **registered** effects
//! (`exchange_effects::effects`), so these tests also prove the stock effect is registered. A quote's
//! reservation is made through SR-1's `PgStockReservations::reserve_in` (nothing else reserves until the binding quote, O-1 A).
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx5_*` test does.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use pos_error::{AppError, AppResult};
use tx4_common::*;
use tx5_common::*;
use uuid::Uuid;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::complete::{CompleteExchange, CompletedExchange};
use xchange::bootstrap::{AppState, exchange_effects};

/// `CompleteExchange` with the ledger double and every registered exchange effect (loyalty, marketing, stock).
fn service(c: &Counter, ledger: &Ledger) -> CompleteExchange {
    let state = AppState::new(Arc::new(common::settings(&[])), c.desk.db.pool.clone());
    CompleteExchange::new(
        c.desk.db.pool.clone(),
        c.files.clone(),
        Arc::new(ledger.clone()),
        Vec::new(),
        Arc::new(exchange_effects::effects(&state)),
    )
}

fn is_code<T: std::fmt::Debug>(got: &AppResult<T>, code: &str) -> bool {
    matches!(got, Err(AppError::Domain(m)) if m.starts_with(code))
}

/// MAIN's USD position at `quantity` (the ledger double moves no stock, so the test sets it).
async fn usd_position(c: &Counter, org: Uuid, quantity: &str) {
    c.desk
        .db
        .exec(&format!(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             VALUES ('{org}', 'MAIN', 'USD', {quantity}, {quantity} * 32) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = EXCLUDED.quantity"
        ))
        .await;
}

/// Reserves the exchange's one quote line (SR-1's store), committed.
async fn reserve(c: &Counter, t: &ExchangeTransaction, now: chrono::DateTime<Utc>) {
    let line = t.lines[0].quote_line_id.expect("a quote line");
    let mut tx = c.desk.db.pool.begin().await.unwrap();
    PgStockReservations::new()
        .reserve_in(
            &mut tx,
            &ReserveLine {
                quote_line_id: line,
                denomination_id: None,
            },
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("reserve_in: {e}"));
    tx.commit().await.unwrap();
}

/// As production: committed on `Ok`, rolled back on `Err`.
async fn complete(
    c: &Counter,
    service: &CompleteExchange,
    id: TransactionId,
    now: chrono::DateTime<Utc>,
) -> AppResult<CompletedExchange> {
    let mut tx = c.desk.db.pool.begin().await.unwrap();
    let got = service.run_in(&mut tx, id, &teller(), now).await;
    if got.is_ok() {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    got
}

/// `status|transaction_id` of the exchange's reservation rows.
async fn rows(c: &Counter, t: &ExchangeTransaction) -> Vec<String> {
    let line = t.lines[0].quote_line_id.expect("a quote line");
    sqlx::query_scalar(
        "SELECT status || '|' || COALESCE(transaction_id::text, '-') FROM stock_reservations \
          WHERE quote_line_id = $1 ORDER BY created_at, id",
    )
    .bind(line)
    .fetch_all(&c.desk.db.pool)
    .await
    .expect("read stock_reservations")
}

#[tokio::test]
async fn s8_a_completion_consumes_its_quotes_reservation() {
    let c = Counter::start().await;
    let now = Utc::now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    usd_position(&c, t.organization_id, "1000").await;
    reserve(&c, &t, now).await;
    let ledger = Ledger::new("3210.00", None);
    let service = service(&c, &ledger);

    complete(&c, &service, t.id, now + Duration::seconds(5))
        .await
        .unwrap_or_else(|e| panic!("complete: {e}"));

    assert_eq!(rows(&c, &t).await, [format!("CONSUMED|{}", t.id.0)]);
    assert_eq!(ledger.calls().len(), 1);
    c.finish().await;
}

#[tokio::test]
async fn s10_a_refused_consume_stops_the_completion_before_the_posting() {
    let c = Counter::start().await;
    let now = Utc::now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    usd_position(&c, t.organization_id, "1000").await;
    reserve(&c, &t, now).await;
    // The hold lapsed and the stock is gone: re-reserving 100 cannot succeed (D-5).
    c.desk
        .db
        .exec(&format!(
            "UPDATE stock_reservations SET expires_at = '{}' WHERE quote_line_id = '{}'",
            (now - Duration::seconds(1)).to_rfc3339(),
            t.lines[0].quote_line_id.unwrap()
        ))
        .await;
    usd_position(&c, t.organization_id, "30").await;
    let ledger = Ledger::new("3210.00", None);
    let service = service(&c, &ledger);

    let got = complete(&c, &service, t.id, now + Duration::seconds(5)).await;

    // #3819: a refused Consume answers stockReserved (no approval can clear it)
    assert!(is_code(&got, "stockReserved"), "{got:?}");
    assert!(
        ledger.calls().is_empty(),
        "Consume runs before post_in: nothing was posted"
    );
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(rows(&c, &t).await, ["RESERVED|-"]);
    c.finish().await;
}

#[tokio::test]
async fn s12_a_second_completion_consumes_nothing_more() {
    let c = Counter::start().await;
    let now = Utc::now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    usd_position(&c, t.organization_id, "1000").await;
    reserve(&c, &t, now).await;
    let ledger = Ledger::new("3210.00", None);
    let service = service(&c, &ledger);
    complete(&c, &service, t.id, now + Duration::seconds(5))
        .await
        .unwrap_or_else(|e| panic!("first: {e}"));

    let again = complete(&c, &service, t.id, now + Duration::seconds(6)).await;

    assert!(
        again.is_err(),
        "a COMPLETED exchange is not completed again"
    );
    assert_eq!(rows(&c, &t).await, [format!("CONSUMED|{}", t.id.0)]);
    assert_eq!(ledger.calls().len(), 1);
    c.finish().await;
}

#[tokio::test]
async fn a_completion_with_no_reservation_completes_as_today() {
    let c = Counter::start().await;
    let now = Utc::now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);
    let service = service(&c, &ledger);

    complete(&c, &service, t.id, now + Duration::seconds(5))
        .await
        .unwrap_or_else(|e| panic!("complete: {e}"));

    assert_eq!(c.status(t.id).await, "COMPLETED");
    assert_eq!(rows(&c, &t).await, Vec::<String>::new());
    assert_eq!(ledger.calls().len(), 1);
    c.finish().await;
}
