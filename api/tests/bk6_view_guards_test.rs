//! Tests for BK-6 follow-up (#1678): view guards in desk_bookings and my_bookings.
//!
//! Behaviour table (§9):
//! - stored phone "+66812345678" -> phoneLast4 = "5678" (unchanged)
//! - stored phone "12" -> phoneLast4 = null
//! - no phone stored -> null (unchanged)
//! - my_bookings as a customer with a linked customer -> their bookings (unchanged)
//! - my_bookings load where who.customer_id is None -> refused, same as my_transactions; no query by ""

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use pos_error::AppError;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::presentation::dto::v3::my_bookings::MyBookingsFilter;
use xchange::presentation::http::v3::my_bookings::MyBookingsView;
use xchange::presentation::http::v3::view::BranchView;

use bk6_common::*;

async fn seed_raw_customer_phone(pool: &PgPool, customer_id: &str, phone: &str) {
    let keys =
        xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
            .expect("the test PII key parses");
    let cipher = xc_kernel::pii::PiiCipher::new(Arc::new(keys));
    let sealed = cipher.encrypt(format!("{customer_id}PHONE").as_bytes(), phone.as_bytes());

    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
         lookup_hash, status) SELECT gen_random_uuid(), organization_id, id, 'PHONE', $2, $3, $4, 'ACTIVE' \
         FROM customers WHERE id = $1",
    )
    .bind(customer_id)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(&[0u8; 32][..])
    .execute(pool)
    .await
    .expect("seed raw customer phone");
}

// ---------------------------------------------------------------------------
// Row 1: stored phone "+66812345678" -> phoneLast4 = "5678" (unchanged)
// ---------------------------------------------------------------------------
#[actix_web::test]
async fn desk_bookings_view_shows_last_4_digits_for_normal_phone() {
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

    let booking_id = "BK-test-row1-phone-normal";
    let qr_hash = Sha256::digest(b"dummy-row1");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-1001",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "DESK",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    seed_raw_customer_phone(&db.pool, CUSTOMER_ALICE_CUST_ID, "+66812345678").await;

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;

    assert_eq!(res.status, StatusCode::OK, "desk_bookings status OK");
    let items = res.body["data"]["items"]
        .as_array()
        .expect("items array in desk_bookings data");
    let item = items
        .iter()
        .find(|i| i["id"].as_str() == Some(booking_id))
        .expect("booking item found");

    assert_eq!(
        item["phoneLast4"], "5678",
        "stored phone '+66812345678' must yield phoneLast4 = '5678'; got {}",
        res.text
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// Row 2: stored phone "12" -> phoneLast4 = null
// ---------------------------------------------------------------------------
#[actix_web::test]
async fn desk_bookings_view_shows_null_for_short_phone() {
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

    let booking_id = "BK-test-row2-phone-short";
    let qr_hash = Sha256::digest(b"dummy-row2");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-1002",
        CUSTOMER_BOB_CUST_ID,
        B01,
        slot_id,
        "DESK",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    seed_raw_customer_phone(&db.pool, CUSTOMER_BOB_CUST_ID, "12").await;

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;

    assert_eq!(res.status, StatusCode::OK, "desk_bookings status OK");
    let items = res.body["data"]["items"]
        .as_array()
        .expect("items array in desk_bookings data");
    let item = items
        .iter()
        .find(|i| i["id"].as_str() == Some(booking_id))
        .expect("booking item found");

    assert!(
        item["phoneLast4"].is_null(),
        "stored phone shorter than 4 characters ('12') must yield phoneLast4 = null; got {:?}",
        item["phoneLast4"]
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// Row 3: no phone stored -> null (unchanged)
// ---------------------------------------------------------------------------
#[actix_web::test]
async fn desk_bookings_view_shows_null_when_no_phone_stored() {
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

    let charlie_cust_id = "CUST-CHARLIE-1";
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, 'CN-003', 'Charlie', 'LIVE', now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(charlie_cust_id)
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("insert charlie customer");

    let booking_id = "BK-test-row3-no-phone";
    let qr_hash = Sha256::digest(b"dummy-row3");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-1003",
        charlie_cust_id,
        B01,
        slot_id,
        "DESK",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    // No phone inserted into customer_identities

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;

    assert_eq!(res.status, StatusCode::OK, "desk_bookings status OK");
    let items = res.body["data"]["items"]
        .as_array()
        .expect("items array in desk_bookings data");
    let item = items
        .iter()
        .find(|i| i["id"].as_str() == Some(booking_id))
        .expect("booking item found");

    assert!(
        item["phoneLast4"].is_null(),
        "customer with no phone stored must yield phoneLast4 = null; got {:?}",
        item["phoneLast4"]
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// Row 4: my_bookings as a customer with a linked customer -> their bookings (unchanged)
// ---------------------------------------------------------------------------
#[actix_web::test]
async fn my_bookings_load_as_customer_with_linked_customer_returns_their_bookings() {
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

    let alice_booking_id = "BK-test-row4-alice-booking";
    let qr_hash = Sha256::digest(b"dummy-row4-alice");
    seed_booking(
        &db.pool,
        alice_booking_id,
        "B261014-1004",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    // Direct BranchView::load check
    let view = MyBookingsView::new(db.pool.clone());
    let who_alice = Actor::new(
        CUSTOMER_ALICE_ID,
        ORG,
        vec![ScopedRole::new(Role::Customer, None, Utc::now(), None)],
        Some(CUSTOMER_ALICE_CUST_ID),
    );

    let data = view
        .load(
            &BranchId::new(B01),
            &who_alice,
            &MyBookingsFilter::default(),
        )
        .await
        .expect("my_bookings load must succeed for customer with linked customer");

    assert_eq!(
        data.bookings.len(),
        1,
        "customer must receive their bookings"
    );
    assert_eq!(data.bookings[0].id, alice_booking_id);

    // Also verify over HTTP route
    let uri = format!("/api/v3/views/my_bookings?branch={B01}");
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(
        res.text.contains(alice_booking_id),
        "my_bookings HTTP response must contain Alice's booking: {}",
        res.text
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// Row 5: my_bookings load where who.customer_id is None -> refused, same as my_transactions; no query by ""
// ---------------------------------------------------------------------------
#[actix_web::test]
async fn my_bookings_load_where_customer_id_is_none_refuses_same_as_my_transactions() {
    let db = TestDb::start().await;

    let view = MyBookingsView::new(db.pool.clone());
    let who_without_cust = Actor::new(
        "user-without-customer",
        ORG,
        vec![ScopedRole::new(Role::Customer, None, Utc::now(), None)],
        None,
    );

    let err = view
        .load(
            &BranchId::new(B01),
            &who_without_cust,
            &MyBookingsFilter::default(),
        )
        .await
        .expect_err("my_bookings load must refuse when who.customer_id is None");

    match err {
        AppError::NotFound { entity, id } => {
            assert_eq!(
                entity, "customer",
                "refusal must name entity 'customer' like my_transactions; got {entity}"
            );
            assert_eq!(id, "", "refusal id must be empty; got {id}");
        }
        other => panic!(
            "expected AppError::NotFound {{ entity: 'customer', id: '' }}, got: {:?}",
            other
        ),
    }

    db.finish().await;
}
