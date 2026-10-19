// Written before the implementation by the test writer gemini/pool-5 (controller claude/e92db6). Do not edit, delete or weaken.

//! Specification tests for BK-6 (#1388, docs/design/blueprint/booking-slots.md §4.4, §9.1):
//! bookingCreate command (`POST /api/bookings`), routes, permissions, and idempotency.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;
use xc_auth::domain::access::{Permission, Role, RoleGrants};

use bk6_common::*;

#[actix_web::test]
async fn booking_create_route_refuses_without_customer_self_or_bookings_desk_permission() {
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
        0,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });

    // 1. Unauthenticated -> 401 signIn
    let res_anon = call(
        &db,
        None,
        Method::POST,
        "/api/bookings",
        Some(payload.clone()),
        Some("k-anon"),
    )
    .await;
    assert_eq!(
        res_anon.status,
        StatusCode::UNAUTHORIZED,
        "expected 401 signIn for anonymous caller; got {} {}",
        res_anon.status,
        res_anon.text
    );

    // 2. Role without permission (FINANCE) -> 400 roleNotAllowed
    let res_finance = call(
        &db,
        Some("FINANCE@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload.clone()),
        Some("k-fin"),
    )
    .await;
    policy_refused(&res_finance, "bookingCreate by FINANCE");

    // 3. Desk role without BookingsDesk in RoleGrants::seed (assert one exists) -> 400 roleNotAllowed
    let desk_candidates = [
        ("ADMIN@XC-B01", Role::Admin),
        ("FINANCE@XC-B01", Role::Finance),
    ];
    assert!(
        desk_candidates
            .iter()
            .any(|(_, r)| !RoleGrants::seed(*r).contains(&Permission::BookingsDesk)),
        "§11.2 must leave some desk role without BookingsDesk"
    );
    let (refused_caller, refused_role) = desk_candidates
        .into_iter()
        .find(|(_, r)| !RoleGrants::seed(*r).contains(&Permission::BookingsDesk))
        .expect("a desk role without BookingsDesk must exist");

    let res_desk = call(
        &db,
        Some(refused_caller),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-refused"),
    )
    .await;
    policy_refused(
        &res_desk,
        &format!("DESK create by {refused_caller} ({refused_role:?})"),
    );

    db.finish().await;
}

#[actix_web::test]
async fn booking_create_refuses_when_bookings_closed() {
    let db = TestDb::start().await;
    // Disable bookings on branch
    sqlx::query("UPDATE branches SET booking_enabled = false WHERE id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("disable bookings");

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
        0,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branch": B01,
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-closed"),
    )
    .await;

    refused_with(
        &res,
        "bookingsClosed",
        "bookingCreate when booking_enabled is false",
    );

    db.finish().await;
}

#[actix_web::test]
async fn app_booking_create_starts_confirmed() {
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
        0,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branch": B01,
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-app-1"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "expected 200 OK for valid APP bookingCreate; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CONFIRMED"), "booking should start CONFIRMED");

    let channel = res.body["sourceChannel"]
        .as_str()
        .or_else(|| res.body["data"]["sourceChannel"].as_str());
    assert_eq!(channel, Some("APP"), "sourceChannel should be APP");

    db.finish().await;
}

#[actix_web::test]
async fn desk_booking_create_with_customer_id_starts_confirmed() {
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
        0,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
        "customerId": CUSTOMER_BOB_CUST_ID,
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-1"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "expected 200 OK for valid DESK bookingCreate with customerId; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(
        status,
        Some("CONFIRMED"),
        "DESK booking should start CONFIRMED"
    );

    let channel = res.body["sourceChannel"]
        .as_str()
        .or_else(|| res.body["data"]["sourceChannel"].as_str());
    assert_eq!(channel, Some("DESK"), "sourceChannel should be DESK");

    db.finish().await;
}

#[actix_web::test]
async fn desk_booking_create_with_display_name_and_phone_creates_customer() {
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
        0,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "SELL",
        "currency": "USD",
        "quantity": 250.0,
        "displayName": "Charlie Customer",
        "phone": "+66812345678",
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-phone-1"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "expected 200 OK for DESK bookingCreate with phone; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CONFIRMED"));

    db.finish().await;
}

#[actix_web::test]
async fn booking_create_idempotency_replay_returns_stored_answer_without_qr_token() {
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
        0,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branch": B01,
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });

    let key = "key-replay-create-1";
    let res1 = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload.clone()),
        Some(key),
    )
    .await;
    assert_eq!(res1.status, StatusCode::OK);

    // Replay with identical key and payload
    let res2 = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some(key),
    )
    .await;
    assert_eq!(res2.status, StatusCode::OK);

    let id1 = res1.body["id"]
        .as_str()
        .or_else(|| res1.body["data"]["id"].as_str());
    let id2 = res2.body["id"]
        .as_str()
        .or_else(|| res2.body["data"]["id"].as_str());
    assert_eq!(id1, id2, "replay must return same booking ID");

    // B7: stored replay answer holds no QR token
    assert!(
        res2.body["qrToken"].is_null() && res2.body["data"]["qrToken"].is_null(),
        "replay response must not return QR token (design B7); got {}",
        res2.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn booking_create_idempotency_conflict_refuses_same_key_different_body() {
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
        0,
        "OPEN",
    )
    .await;

    let key = "key-conflict-create-1";
    let payload1 = json!({
        "branch": B01,
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });
    let res1 = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload1),
        Some(key),
    )
    .await;
    assert_eq!(res1.status, StatusCode::OK);

    let payload2 = json!({
        "branch": B01,
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 200.0, // different quantity
    });
    let res2 = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload2),
        Some(key),
    )
    .await;
    assert_eq!(
        res2.status,
        StatusCode::CONFLICT,
        "same key with different body must return 409 idempotencyConflict; got {} {}",
        res2.status,
        res2.text
    );
    assert_eq!(res2.body["error"]["code"], "idempotencyConflict");

    db.finish().await;
}

#[actix_web::test]
async fn booking_create_refuses_when_slot_capacity_full() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    // Slot with capacity 1, already 1 reserved
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(1),
        now + Duration::hours(2),
        1,
        1,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branch": B01,
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-full-1"),
    )
    .await;
    refused_with(&res, "capacityFull", "create on full slot");

    db.finish().await;
}

#[actix_web::test]
async fn booking_create_desk_by_branch_manager_succeeds_under_seed() {
    // Under RBAC §11.2, BranchManager holds BookingsDesk (Customers: Full).
    assert!(
        RoleGrants::seed(Role::BranchManager).contains(&Permission::BookingsDesk),
        "§11.2: BRANCH_MANAGER holds BookingsDesk (CUSTOMERS FULL)"
    );
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
        0,
        "OPEN",
    )
    .await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
        "customerId": CUSTOMER_BOB_CUST_ID,
    });

    let res = call(
        &db,
        Some("BRANCH_MANAGER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-bm"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "expected 200 OK for valid DESK bookingCreate by BRANCH_MANAGER; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(
        status,
        Some("CONFIRMED"),
        "DESK booking by BRANCH_MANAGER should start CONFIRMED"
    );

    let channel = res.body["sourceChannel"]
        .as_str()
        .or_else(|| res.body["data"]["sourceChannel"].as_str());
    assert_eq!(channel, Some("DESK"), "sourceChannel should be DESK");

    db.finish().await;
}
