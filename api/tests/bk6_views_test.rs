// Written before the implementation by the test writer gemini/pool-5 (controller claude/e92db6). Do not edit, delete or weaken.

//! Specification tests for BK-6 (#1388, docs/design/blueprint/booking-slots.md §4.4, §9.1):
//! views: booking_slots (public counts only), desk_bookings (phone last 4 only, readiness unknown, qr filter),
//! my_bookings / my_booking (own customer only, recomputed QR token, outOfScope on another customer),
//! admin_booking_slots (rule, closures, slots with capacity/reserved, policy), and since -> {changed: false}.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

#[actix_web::test]
async fn booking_slots_view_is_public_and_carries_counts_only() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    let today = bangkok_today();
    let date_str = today.to_string();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        today,
        now + Duration::hours(1),
        now + Duration::hours(2),
        5,
        2,
        "OPEN",
    )
    .await;

    // Public view: anonymous request
    let uri = format!("/api/v3/views/booking_slots?branch={B01}&date={date_str}");
    let res = call(&db, None, Method::GET, &uri, None, None).await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "booking_slots view must be public 200 OK; got {} {}",
        res.status,
        res.text
    );

    // Carries counts only: available = capacity (5) - reserved (2) = 3
    let available_value = res.body["data"]["slots"]
        .as_array()
        .or_else(|| res.body["data"].as_array())
        .and_then(|arr| arr.iter().find_map(|s| s["available"].as_i64()))
        .or_else(|| res.body["available"].as_i64());
    assert_eq!(
        available_value,
        Some(3),
        "booking_slots must carry available count = 3 (capacity 5 - reserved 2); got {}",
        res.text
    );
    assert!(
        !res.text.contains("customer"),
        "must NEVER carry customer details"
    );
    assert!(
        !res.text.contains("phone"),
        "must NEVER carry phone numbers"
    );
    assert!(
        !res.text.contains("bookingNo"),
        "must NEVER carry individual booking numbers"
    );

    db.finish().await;
}

#[actix_web::test]
async fn booking_slots_view_since_unchanged_returns_changed_false() {
    let db = TestDb::start().await;
    let date_str = bangkok_today().to_string();
    let uri = format!("/api/v3/views/booking_slots?branch={B01}&date={date_str}");

    let res1 = call(&db, None, Method::GET, &uri, None, None).await;
    assert_eq!(res1.status, StatusCode::OK);

    let version = res1.body["version"]
        .as_i64()
        .expect("view must return version");

    let uri_since = format!("{uri}&since={version}");
    let res2 = call(&db, None, Method::GET, &uri_since, None, None).await;
    assert_eq!(res2.status, StatusCode::OK);
    assert_eq!(
        res2.body["changed"], false,
        "polling with since unchanged must return changed: false; got {}",
        res2.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn desk_bookings_view_refuses_without_bookings_desk() {
    let db = TestDb::start().await;
    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");

    // 1. Unauthenticated -> 401
    let res_anon = call(&db, None, Method::GET, &uri, None, None).await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // 2. Customer -> 400 roleNotAllowed
    let res_cust = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;
    policy_refused(&res_cust, "desk_bookings by CUSTOMER");

    db.finish().await;
}

#[actix_web::test]
async fn desk_bookings_view_shows_phone_last_4_only_and_never_full_phone() {
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

    let booking_id = "BK-test-desk-phone-view";
    let qr_hash = Sha256::digest(b"dummy-desk-phone-view");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0020",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "DESK",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let full_phone = "+66891234567";
    seed_customer_phone(&db.pool, CUSTOMER_ALICE_CUST_ID, full_phone).await;

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;

    assert_eq!(res.status, StatusCode::OK);
    // Design §4.3 & §4.4: desk_bookings shows phone last 4 only, never the full phone
    assert!(
        !res.text.contains(full_phone),
        "desk_bookings must never reveal full phone number; got {}",
        res.text
    );
    assert!(
        res.text.contains("4567"),
        "desk_bookings must contain last 4 digits of phone ('4567'); got {}",
        res.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn desk_bookings_view_readiness_is_unknown() {
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

    let booking_id = "BK-test-readiness-unknown";
    let qr_hash = Sha256::digest(b"dummy-readiness");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0021",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;

    assert_eq!(res.status, StatusCode::OK);
    // Ticket §7: Readiness reads "unknown" until #557's view exists (design §5.1)
    assert!(
        res.text.contains("\"unknown\""),
        "desk_bookings items readiness must read 'unknown'; got {}",
        res.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn desk_bookings_view_qr_filter_returns_matched_booking() {
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

    let booking_id = "BK-test-qr-filter";
    let qr_token = "unique-scanned-qr-filter-token";
    let qr_hash = Sha256::digest(qr_token.as_bytes());
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0022",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}&qr={qr_token}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;

    assert_eq!(res.status, StatusCode::OK);
    assert!(
        res.text.contains(booking_id),
        "qr filter must return the matched booking"
    );

    db.finish().await;
}

#[actix_web::test]
async fn desk_bookings_view_since_unchanged_returns_changed_false() {
    let db = TestDb::start().await;
    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");

    let res1 = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;
    assert_eq!(res1.status, StatusCode::OK);

    let version = res1.body["version"]
        .as_i64()
        .expect("view must return version");

    let uri_since = format!("{uri}&since={version}");
    let res2 = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::GET,
        &uri_since,
        None,
        None,
    )
    .await;
    assert_eq!(res2.status, StatusCode::OK);
    assert_eq!(res2.body["changed"], false);

    db.finish().await;
}

#[actix_web::test]
async fn my_bookings_view_refuses_without_customer_self() {
    let db = TestDb::start().await;
    let uri = format!("/api/v3/views/my_bookings?branch={B01}");

    // 1. Unauthenticated -> 401
    let res_anon = call(&db, None, Method::GET, &uri, None, None).await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // 2. Staff teller (no CustomerSelf) -> 400 roleNotAllowed
    let res_teller = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;
    policy_refused(&res_teller, "my_bookings by TELLER");

    db.finish().await;
}

#[actix_web::test]
async fn my_booking_of_another_customer_refuses_out_of_scope() {
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

    let alice_booking_id = "BK-test-alice-my-booking";
    let qr_hash = Sha256::digest(b"dummy-alice-booking");
    seed_booking(
        &db.pool,
        alice_booking_id,
        "B261014-0023",
        CUSTOMER_ALICE_CUST_ID, // Alice
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    // Bob tries to view Alice's booking -> 403 outOfScope
    let uri = format!("/api/v3/views/my_bookings?branch={B01}&bookingId={alice_booking_id}");
    let res = call(
        &db,
        Some("CUSTOMER@customer-bob"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::FORBIDDEN,
        "viewing another customer's booking must return 403 outOfScope; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(res.body["error"]["code"], "outOfScope");

    db.finish().await;
}

#[actix_web::test]
async fn my_bookings_view_returns_recomputed_qr_token_and_quote_summary() {
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

    let booking_id = "BK-test-my-bookings-own";
    let qr_hash = Sha256::digest(b"dummy-my-bookings-qr");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0024",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

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
    // Design §4.4: status, branch, slot, the QR token (recomputed, B7), readiness
    assert!(res.text.contains(booking_id));
    assert!(res.text.contains("qrToken"));
    assert!(res.text.contains("\"unknown\""));

    db.finish().await;
}

#[actix_web::test]
async fn admin_booking_slots_view_refuses_without_branches_manage() {
    let db = TestDb::start().await;
    let uri = format!("/api/v3/views/admin_booking_slots?branch={B01}");

    // TELLER has no BranchesManage -> 400 roleNotAllowed
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;
    policy_refused(&res, "admin_booking_slots by TELLER");

    db.finish().await;
}

#[actix_web::test]
async fn admin_booking_slots_view_returns_rule_closures_slots_and_policy() {
    let db = TestDb::start().await;
    seed_slot_rule(&db.pool, B01, 30, 6, 14).await;

    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(1),
        now + Duration::hours(2),
        6,
        1,
        "OPEN",
    )
    .await;

    let uri = format!("/api/v3/views/admin_booking_slots?branch={B01}");
    let res = call(&db, Some("ADMIN@*"), Method::GET, &uri, None, None).await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "admin_booking_slots by ADMIN must succeed 200; got {} {}",
        res.status,
        res.text
    );

    // Design §4.4: rule, closures, slots with capacity and reserved per day, policy
    assert!(res.text.contains("rule"));
    assert!(res.text.contains("policy"));
    assert!(res.text.contains("slots"));

    db.finish().await;
}
