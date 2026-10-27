// Written before the implementation by the ticket author claude/e2926a49 (sub-controller
// XC-BOOKING-ESTIMATE under the xchange BE lane controller claude/bc8fd29a). Do not edit, delete or weaken.

//! Up-front specification tests for BE-2 (design `docs/design/blueprint/marketing-campaigns.md` §6.4,
//! E-10 and behaviour row 21): every `my_bookings` item carries the booking's intent, so C03 can ask
//! `planner_preview` for one booking's estimate.
//!
//! - `currency` = the booking's `requested_currency`;
//! - `customerSide` = its `shop_side` seen from the customer, through `CustomerSide::from_shop`
//!   (`SELL` ⇒ `CUSTOMER_BUY`, `BUY` ⇒ `CUSTOMER_SELL`);
//! - `quantity` = its `requested_quantity`, as stored (`numeric(20,4)`, so `100` reads `"100.0000"`).
//!
//! `quoteSummary` keeps its text.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use bk6_common::*;

/// A CONFIRMED booking of Alice in a fresh slot of today, with the given intent; answers its id.
async fn booking_with_intent(
    pool: &PgPool,
    id: &str,
    no: &str,
    shop_side: &str,
    currency: &str,
    quantity: &str,
) -> String {
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(1),
        now + Duration::hours(2),
        5,
        1,
        "OPEN",
    )
    .await;
    let qr = Sha256::digest(id.as_bytes());
    seed_booking(
        pool,
        id,
        no,
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr,
    )
    .await;
    sqlx::query(
        "UPDATE bookings SET shop_side = $2, requested_currency = $3, \
         requested_quantity = $4::numeric WHERE id = $1",
    )
    .bind(id)
    .bind(shop_side)
    .bind(currency)
    .bind(quantity)
    .execute(pool)
    .await
    .expect("set the booking's intent");
    id.to_owned()
}

/// Alice's booking `id` as `my_bookings` answers it.
async fn alices(db: &TestDb, id: &str) -> Value {
    let uri = format!("/api/v3/views/my_bookings?branch={B01}&bookingId={id}");
    let res = call(
        db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "my_bookings: {}", res.text);
    let body: Value = serde_json::from_str(&res.text).expect("JSON");
    body["data"]["bookings"]
        .as_array()
        .and_then(|b| b.iter().find(|r| r["id"] == id).cloned())
        .unwrap_or_else(|| panic!("booking {id} in my_bookings: {}", res.text))
}

/// Row 21: the shop SELLS 100 USD ⇒ the customer buys: `CUSTOMER_BUY`, `USD`, `100.0000`.
#[actix_web::test]
async fn c03_a_shop_sell_booking_carries_customer_buy_currency_and_quantity() {
    let db = TestDb::start().await;
    let id = booking_with_intent(
        &db.pool,
        "BK-c03-intent-sell",
        "B-C03-0901",
        "SELL",
        "USD",
        "100",
    )
    .await;

    let row = alices(&db, &id).await;
    assert_eq!(row["customerSide"], "CUSTOMER_BUY", "{row}");
    assert_eq!(row["currency"], "USD", "{row}");
    assert_eq!(
        row["quantity"], "100.0000",
        "as stored, numeric(20,4): {row}"
    );
    assert_eq!(
        row["quoteSummary"], "เรทสุดท้ายยืนยันเมื่อทำรายการ",
        "quoteSummary stays as it is: {row}"
    );
    db.finish().await;
}

/// Row 21, the other side: the shop BUYS 250.5 EUR ⇒ the customer sells: `CUSTOMER_SELL`, `EUR`, `250.5000`.
#[actix_web::test]
async fn c03_a_shop_buy_booking_carries_customer_sell_currency_and_quantity() {
    let db = TestDb::start().await;
    let id = booking_with_intent(
        &db.pool,
        "BK-c03-intent-buy",
        "B-C03-0902",
        "BUY",
        "EUR",
        "250.5",
    )
    .await;

    let row = alices(&db, &id).await;
    assert_eq!(row["customerSide"], "CUSTOMER_SELL", "{row}");
    assert_eq!(row["currency"], "EUR", "{row}");
    assert_eq!(
        row["quantity"], "250.5000",
        "as stored, numeric(20,4): {row}"
    );
    db.finish().await;
}
