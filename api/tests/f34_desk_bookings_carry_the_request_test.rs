//! F34a: desk_bookings rows carry the booking's side, currency and quantity (#6333).
//!
//! A booking seeded with shop_side 'SELL', USD, 300 reads back from
//! GET /view desk_bookings with shopSide: "SELL", currency: "USD", quantity equal to "300.0000".

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

fn row_of<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => {
            if map.get("id").and_then(Value::as_str) == Some(id) {
                return Some(value);
            }
            map.values().find_map(|v| row_of(v, id))
        }
        Value::Array(items) => items.iter().find_map(|v| row_of(v, id)),
        _ => None,
    }
}

#[actix_web::test]
async fn f34_desk_bookings_carry_the_request() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
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

    let booking_id = "BK-f34-request-test";
    let qr_hash = Sha256::digest(b"dummy-f34-request");

    // A booking seeded with shop_side 'SELL', USD, 300
    sqlx::query(
        "INSERT INTO bookings ( \
             id, organization_id, branch_id, booking_no, customer_id, slot_id, \
             shop_side, requested_currency, requested_quantity, source_channel, \
             status, qr_token_hash, qr_version, created_by \
         ) VALUES ( \
             $1, $2::uuid, $3, $4, $5, $6, \
             'SELL', 'USD', 300, 'APP', \
             'CONFIRMED', $7, 1, 'test' \
         )",
    )
    .bind(booking_id)
    .bind(ORG)
    .bind(B01)
    .bind("B261105-0034")
    .bind(CUSTOMER_ALICE_CUST_ID)
    .bind(slot_id)
    .bind(&qr_hash[..])
    .execute(&db.pool)
    .await
    .expect("seed booking");

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "GET desk_bookings: {}",
        res.text
    );

    let row = row_of(&res.body, booking_id).unwrap_or_else(|| {
        panic!(
            "booking {booking_id} not found in desk_bookings: {}",
            res.text
        )
    });

    assert_eq!(
        row["shopSide"], "SELL",
        "shopSide must be SELL: {}",
        res.text
    );
    assert_eq!(row["currency"], "USD", "currency must be USD: {}", res.text);
    assert_eq!(
        row["quantity"], "300.0000",
        "quantity must be 300.0000: {}",
        res.text
    );

    db.finish().await;
}
