//! Up-front tests for SR-4 (B05: the `branch_stock` view; design `docs/design/blueprint/stock-reservation.md`
//! D-10, D-11, D-13, D-17, §4.2 S14, S17, S22). Written by the B05 sub-controller claude/6530d490 on
//! `feat/b05-stock` before the implementation. **Do not edit, delete or weaken.**
//!
//! They call the view's `read` directly (`presentation/http/v3/views/branch_stock.rs`); the HTTP core is the
//! view table's, unchanged. The committed `web/src/shared/api/generated/branch_stock.ts` is checked by
//! `v3_types_test`.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx4_*` test does.

mod quote_common;
mod tx4_common;

use std::str::FromStr;

use chrono::{DateTime, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use tx4_common::*;
use uuid::Uuid;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::presentation::dto::v3::branch_stock::{
    BranchStockData, BranchStockRow, TransferPrefill,
};
use xchange::presentation::http::v3::branch_stock::read;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|_| panic!("a decimal string on the wire: {s:?}"))
}

async fn org(desk: &Desk) -> Uuid {
    let q = desk
        .db
        .text("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
        .await
        .expect("an organisation");
    Uuid::parse_str(&q).unwrap()
}

async fn position(desk: &Desk, org: Uuid, currency: &str, quantity: &str) {
    desk.db
        .exec(&format!(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             VALUES ('{org}', 'MAIN', '{currency}', {quantity}, {quantity} * 32) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = EXCLUDED.quantity"
        ))
        .await;
}

async fn floor(desk: &Desk, org: Uuid, currency: &str, min: &str) {
    desk.db
        .exec(&format!(
            "INSERT INTO branch_stock_floors (organization_id, branch_id, currency, min_quantity, updated_by, \
             updated_at) VALUES ('{org}', 'MAIN', '{currency}', {min}, '{}', now())",
            Uuid::new_v4()
        ))
        .await;
}

/// A SELL quote of `quantity` USD at MAIN, its line reserved (SR-1's store).
async fn hold(desk: &Desk, quantity: &str, now: DateTime<Utc>) {
    let q = desk.quote("MAIN", Side::Sell, quantity, None, now).await;
    let mut tx = desk.db.pool.begin().await.unwrap();
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

async fn view(desk: &Desk, org: Uuid, now: DateTime<Utc>) -> BranchStockData {
    read(&desk.db.pool, org, &BranchId::new("MAIN"), now)
        .await
        .unwrap_or_else(|e| panic!("branch_stock: {e}"))
}

fn row<'a>(data: &'a BranchStockData, currency: &str) -> &'a BranchStockRow {
    data.rows
        .iter()
        .find(|r| r.currency == currency)
        .unwrap_or_else(|| panic!("a {currency} row in {data:?}"))
}

/// `(onHand, reserved, available)` as values.
fn figures(r: &BranchStockRow) -> (Decimal, Decimal, Decimal) {
    (d(&r.on_hand), d(&r.reserved), d(&r.available))
}

#[tokio::test]
async fn s14_on_hand_reserved_available_floor_and_low_per_currency() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let org = org(&desk).await;
    position(&desk, org, "USD", "100").await;
    position(&desk, org, "EUR", "50").await;
    floor(&desk, org, "USD", "30").await;
    hold(&desk, "60", now).await;

    let data = view(&desk, org, now).await;

    assert_eq!(data.branch_id, "MAIN");
    let usd = row(&data, "USD");
    assert_eq!(figures(usd), (d("100"), d("60"), d("40")));
    assert_eq!(usd.floor.as_deref().map(d), Some(d("30")));
    assert!(!usd.low, "40 available is not below 30");
    assert_eq!(usd.transfer_prefill, None);

    let eur = row(&data, "EUR");
    assert_eq!(figures(eur), (d("50"), d("0"), d("50")));
    assert_eq!(eur.floor, None, "no floor entered (D-11)");
    assert!(!eur.low, "no floor ⇒ never low (S14)");
    assert_eq!(eur.transfer_prefill, None);

    let order: Vec<&str> = data.rows.iter().map(|r| r.currency.as_str()).collect();
    let mut sorted = order.clone();
    sorted.sort_by_key(|c| (*c != "THB", c.to_string()));
    assert_eq!(order, sorted, "THB first, then A–Z");
    desk.db.finish().await;
}

#[tokio::test]
async fn s17_s22_low_compares_the_floor_with_available_and_links_a_transfer() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let org = org(&desk).await;
    position(&desk, org, "USD", "80").await;
    floor(&desk, org, "USD", "50").await;
    hold(&desk, "40", now).await;

    let usd = row(&view(&desk, org, now).await, "USD").clone();

    assert_eq!(figures(&usd), (d("80"), d("40"), d("40")));
    assert!(
        usd.low,
        "available 40 < floor 50 although on hand 80 is not (D-13)"
    );
    assert_eq!(
        usd.transfer_prefill,
        Some(TransferPrefill {
            branch_id: "MAIN".to_owned(),
            currency: "USD".to_owned(),
        })
    );
    let transfers = desk.db.count("SELECT count(*) FROM cash_transfers").await;
    assert_eq!(transfers, 0, "nothing is created (D-17)");
    desk.db.finish().await;
}

#[tokio::test]
async fn a_floor_with_nothing_on_hand_is_a_low_row() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let org = org(&desk).await;
    floor(&desk, org, "JPY", "1000").await;

    let jpy = row(&view(&desk, org, now).await, "JPY").clone();

    assert_eq!(figures(&jpy), (d("0"), d("0"), d("0")));
    assert!(jpy.low);
    assert!(jpy.transfer_prefill.is_some());
    desk.db.finish().await;
}

#[tokio::test]
async fn an_expired_hold_is_not_reserved_in_the_view() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let org = org(&desk).await;
    position(&desk, org, "USD", "100").await;
    hold(&desk, "60", now).await;

    // desk quotes live 300 s (exchange_settings), so the hold lapses with them
    let usd = row(&view(&desk, org, later(now, 301)).await, "USD").clone();

    assert_eq!(figures(&usd), (d("100"), d("0"), d("100")));
    desk.db.finish().await;
}
