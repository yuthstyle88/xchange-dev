// Written before the implementation by the test writer gemini/pool-5 (controller claude/e92db6). Do not edit, delete or weaken.

//! Specification tests for BK-6 (#1388, docs/design/blueprint/booking-slots.md §4.4, §9.1):
//! booking lifecycle commands: confirm (`POST /api/bookings/{id}/confirm`),
//! cancel (`POST /api/bookings/{id}/cancel`), reschedule (`POST /api/bookings/{id}/reschedule`),
//! and qr reissue (`POST /api/bookings/{id}/qr`).

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

#[actix_web::test]
async fn confirm_route_refuses_without_customer_self() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-confirm-perm";
    let qr_hash = Sha256::digest(b"dummy-qr-confirm");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0010",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "PARTNER",
        "PENDING_CUSTOMER_CONFIRMATION",
        &qr_hash,
    )
    .await;

    // confirm_expires_at in future
    sqlx::query("UPDATE bookings SET confirm_expires_at = now() + interval '1 hour' WHERE id = $1")
        .bind(booking_id)
        .execute(&db.pool)
        .await
        .expect("update confirm_expires_at");

    let uri = format!("/api/bookings/{booking_id}/confirm");

    // 1. Unauthenticated -> 401
    let res_anon = call(&db, None, Method::POST, &uri, None, Some("k-conf-anon")).await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // 2. Staff teller (no CustomerSelf) -> 400 roleNotAllowed
    let res_teller = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        None,
        Some("k-conf-teller"),
    )
    .await;
    policy_refused(&res_teller, "confirm by TELLER");

    db.finish().await;
}

#[actix_web::test]
async fn confirm_of_another_customer_booking_refuses_out_of_scope() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-confirm-alice";
    let qr_hash = Sha256::digest(b"dummy-qr-alice");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0011",
        CUSTOMER_ALICE_CUST_ID, // Alice's booking
        B01,
        slot_id,
        "PARTNER",
        "PENDING_CUSTOMER_CONFIRMATION",
        &qr_hash,
    )
    .await;

    sqlx::query("UPDATE bookings SET confirm_expires_at = now() + interval '1 hour' WHERE id = $1")
        .bind(booking_id)
        .execute(&db.pool)
        .await
        .expect("update confirm_expires_at");

    let uri = format!("/api/bookings/{booking_id}/confirm");

    // Bob trying to confirm Alice's booking -> 403 outOfScope
    let res = call(
        &db,
        Some("CUSTOMER@customer-bob"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-conf-bob"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::FORBIDDEN,
        "confirming another customer's booking must return 403 outOfScope; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(res.body["error"]["code"], "outOfScope");

    db.finish().await;
}

#[actix_web::test]
async fn confirm_after_confirm_expires_at_refuses_booking_expired() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-confirm-expired";
    let qr_hash = Sha256::digest(b"dummy-qr-expired");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0012",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "PARTNER",
        "PENDING_CUSTOMER_CONFIRMATION",
        &qr_hash,
    )
    .await;

    // confirm_expires_at in past
    sqlx::query(
        "UPDATE bookings SET confirm_expires_at = now() - interval '10 minutes' WHERE id = $1",
    )
    .bind(booking_id)
    .execute(&db.pool)
    .await
    .expect("update confirm_expires_at to past");

    let uri = format!("/api/bookings/{booking_id}/confirm");
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-conf-exp"),
    )
    .await;

    refused_with(&res, "bookingExpired", "confirm after expiry");

    db.finish().await;
}

#[actix_web::test]
async fn cancel_route_refuses_without_permission() {
    let db = TestDb::start().await;
    let booking_id = "BK-test-cancel-perm";
    let uri = format!("/api/bookings/{booking_id}/cancel");

    // Unauthenticated -> 401
    let res_anon = call(
        &db,
        None,
        Method::POST,
        &uri,
        Some(json!({"reason": "test"})),
        Some("k-c-anon"),
    )
    .await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // FINANCE role (neither CustomerSelf nor BookingsDesk) -> 400 roleNotAllowed
    let res_fin = call(
        &db,
        Some("FINANCE@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({"reason": "test"})),
        Some("k-c-fin"),
    )
    .await;
    policy_refused(&res_fin, "cancel by FINANCE");

    db.finish().await;
}

#[actix_web::test]
async fn teller_cancel_without_reason_refuses_validation_failed() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-teller-no-reason";
    let qr_hash = Sha256::digest(b"dummy-qr-cancel-1");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0013",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/cancel");
    // Staff cancels with empty/missing reason
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({})),
        Some("k-c-no-reason"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::BAD_REQUEST,
        "teller cancel without reason must return 400 validationFailed; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(res.body["error"]["code"], "validationFailed");

    db.finish().await;
}

#[actix_web::test]
async fn customer_cancel_before_cutoff_releases_place() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    // Slot starts 3 hours from now (cutoff is 60 minutes)
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(3),
        now + Duration::hours(4),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-cust-cancel-ok";
    let qr_hash = Sha256::digest(b"dummy-qr-cancel-ok");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0014",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/cancel");
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-c-ok"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "customer cancel before cutoff must succeed 200; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CANCELLED"));

    let (reserved,): (i32,) = sqlx::query_as("SELECT reserved FROM booking_slots WHERE id = $1")
        .bind(slot_id)
        .fetch_one(&db.pool)
        .await
        .expect("read slot reserved count");
    assert_eq!(reserved, 0, "slot reserved count must be decremented to 0");

    db.finish().await;
}

#[actix_web::test]
async fn customer_cancel_after_cutoff_refuses_cancel_cutoff_passed() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    // Slot starts 30 minutes from now (cutoff is 60 minutes)
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(30),
        now + Duration::minutes(60),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-cust-cancel-late";
    let qr_hash = Sha256::digest(b"dummy-qr-cancel-late");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0015",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/cancel");
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-c-late"),
    )
    .await;

    refused_with(&res, "cancelCutoffPassed", "customer cancel after cutoff");

    db.finish().await;
}

#[actix_web::test]
async fn reschedule_route_refuses_without_permission() {
    let db = TestDb::start().await;
    let booking_id = "BK-test-resched-perm";
    let uri = format!("/api/bookings/{booking_id}/reschedule");
    let payload = json!({ "slotId": Uuid::new_v4() });

    // Unauthenticated -> 401
    let res_anon = call(
        &db,
        None,
        Method::POST,
        &uri,
        Some(payload.clone()),
        Some("k-r-anon"),
    )
    .await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // FINANCE -> 400 roleNotAllowed
    let res_fin = call(
        &db,
        Some("FINANCE@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-r-fin"),
    )
    .await;
    policy_refused(&res_fin, "reschedule by FINANCE");

    db.finish().await;
}

#[actix_web::test]
async fn reschedule_to_full_slot_refuses_capacity_full() {
    let db = TestDb::start().await;
    let now = Utc::now();
    let slot1_id = Uuid::new_v4();
    let slot2_id = Uuid::new_v4();

    seed_slot(
        &db.pool,
        slot1_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    // Slot 2 is FULL (capacity 2, reserved 2)
    seed_slot(
        &db.pool,
        slot2_id,
        B01,
        bangkok_today(),
        now + Duration::hours(4),
        now + Duration::hours(5),
        2,
        2,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-resched-full";
    let qr_hash = Sha256::digest(b"dummy-qr-resched-full");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0016",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot1_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/reschedule");
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01, "slotId": slot2_id })),
        Some("k-r-full"),
    )
    .await;

    refused_with(&res, "capacityFull", "reschedule to full slot");

    db.finish().await;
}

#[actix_web::test]
async fn reschedule_to_available_slot_moves_place() {
    let db = TestDb::start().await;
    let now = Utc::now();
    let slot1_id = Uuid::new_v4();
    let slot2_id = Uuid::new_v4();

    seed_slot(
        &db.pool,
        slot1_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    seed_slot(
        &db.pool,
        slot2_id,
        B01,
        bangkok_today(),
        now + Duration::hours(4),
        now + Duration::hours(5),
        5,
        0,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-resched-ok";
    let qr_hash = Sha256::digest(b"dummy-qr-resched-ok");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0017",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot1_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/reschedule");
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01, "slotId": slot2_id })),
        Some("k-r-ok"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "reschedule to available slot must succeed 200; got {} {}",
        res.status,
        res.text
    );

    let new_slot = res.body["slotId"]
        .as_str()
        .or_else(|| res.body["data"]["slotId"].as_str());
    assert_eq!(new_slot, Some(slot2_id.to_string().as_str()));

    let (res1,): (i32,) = sqlx::query_as("SELECT reserved FROM booking_slots WHERE id = $1")
        .bind(slot1_id)
        .fetch_one(&db.pool)
        .await
        .expect("read old slot reserved count");
    assert_eq!(res1, 0, "old slot reserved count must decrease by 1 to 0");

    let (res2,): (i32,) = sqlx::query_as("SELECT reserved FROM booking_slots WHERE id = $1")
        .bind(slot2_id)
        .fetch_one(&db.pool)
        .await
        .expect("read new slot reserved count");
    assert_eq!(res2, 1, "new slot reserved count must increase by 1 to 1");

    db.finish().await;
}

#[actix_web::test]
async fn qr_reissue_route_refuses_without_customer_self_or_bookings_desk() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-qr-perm";
    let qr_hash = Sha256::digest(b"dummy-qr-perm");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0018a",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/qr");

    // 1. Unauthenticated -> 401
    let res_anon = call(&db, None, Method::POST, &uri, None, Some("k-qr-anon")).await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // 2. Role without permission (FINANCE) -> 400 roleNotAllowed
    let res_fin = call(
        &db,
        Some("FINANCE@XC-B01"),
        Method::POST,
        &uri,
        None,
        Some("k-qr-fin"),
    )
    .await;
    policy_refused(&res_fin, "bookingQrReissue by FINANCE");

    // 3. Other customer (Bob) -> 403 outOfScope
    let res_bob = call(
        &db,
        Some("CUSTOMER@customer-bob"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-qr-bob"),
    )
    .await;
    assert_eq!(
        res_bob.status,
        StatusCode::FORBIDDEN,
        "bookingQrReissue by other customer must be 403 outOfScope; got {} {}",
        res_bob.status,
        res_bob.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn qr_reissue_invalidates_old_token_and_returns_new_token() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "00000000-0000-4000-8000-000000000018";
    let qr_hash = Sha256::digest(b"old-qr-token-1");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0018",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/qr");
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-reissue-1"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "QR reissue must succeed 200; got {} {}",
        res.status,
        res.text
    );

    let new_qr = res.body["qrToken"]
        .as_str()
        .or_else(|| res.body["data"]["qrToken"].as_str());
    assert!(new_qr.is_some(), "reissue response must carry new qrToken");

    // Old token finds nothing (ticket §9, design §9.1): check-in with old token fails qrMismatch
    let checkin_uri = format!("/api/bookings/{booking_id}/check-in");
    let checkin_res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &checkin_uri,
        Some(json!({ "qrToken": "old-qr-token-1" })),
        Some("k-checkin-old-token"),
    )
    .await;
    refused_with(
        &checkin_res,
        "qrMismatch",
        "check-in with invalidated old QR token",
    );

    // And desk_bookings?qr= lookup finds nothing
    let lookup_uri = format!("/api/v3/views/desk_bookings?branch={B01}&qr=old-qr-token-1");
    let lookup_res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::GET,
        &lookup_uri,
        None,
        None,
    )
    .await;
    assert_eq!(lookup_res.status, StatusCode::OK);
    assert!(
        !lookup_res.text.contains(booking_id),
        "old QR token must find nothing in desk_bookings"
    );

    db.finish().await;
}

#[actix_web::test]
async fn lifecycle_commands_idempotency_replay_and_conflict() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let slot2_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(3),
        now + Duration::hours(4),
        5,
        3,
        "OPEN",
    )
    .await;
    seed_slot(
        &db.pool,
        slot2_id,
        B01,
        bangkok_today(),
        now + Duration::hours(5),
        now + Duration::hours(6),
        5,
        0,
        "OPEN",
    )
    .await;

    // 1. Cancel target booking
    let b_cancel = "00000000-0000-4000-8000-00000000001a";
    let qr_cancel = Sha256::digest(b"dummy-life-cancel");
    seed_booking(
        &db.pool,
        b_cancel,
        "B261014-0019a",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_cancel,
    )
    .await;

    // 2. Confirm target booking
    let b_confirm = "00000000-0000-4000-8000-00000000001b";
    let qr_confirm = Sha256::digest(b"dummy-life-confirm");
    seed_booking(
        &db.pool,
        b_confirm,
        "B261014-0019b",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "PARTNER",
        "PENDING_CUSTOMER_CONFIRMATION",
        &qr_confirm,
    )
    .await;
    sqlx::query("UPDATE bookings SET confirm_expires_at = now() + interval '1 hour' WHERE id = $1")
        .bind(b_confirm)
        .execute(&db.pool)
        .await
        .expect("update confirm_expires_at");

    // 3. Reschedule target booking
    let b_resched = "00000000-0000-4000-8000-00000000001c";
    let qr_resched = Sha256::digest(b"dummy-life-reschedule");
    seed_booking(
        &db.pool,
        b_resched,
        "B261014-0019c",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_resched,
    )
    .await;

    // 4. QR reissue target booking
    let b_qr = "00000000-0000-4000-8000-00000000001d";
    let qr_reissue = Sha256::digest(b"dummy-life-reissue");
    seed_booking(
        &db.pool,
        b_qr,
        "B261014-0019d",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_reissue,
    )
    .await;

    let cases = vec![
        (
            "bookingCancel",
            Method::POST,
            format!("/api/bookings/{b_cancel}/cancel"),
            Some("CUSTOMER@customer-alice"),
            Some(json!({ "branch": B01 })),
            Some(json!({ "branch": B01, "reason": "changed" })),
        ),
        (
            "bookingConfirm",
            Method::POST,
            format!("/api/bookings/{b_confirm}/confirm"),
            Some("CUSTOMER@customer-alice"),
            Some(json!({ "branch": B01 })),
            Some(json!({ "branch": B01, "note": "different" })),
        ),
        (
            "bookingReschedule",
            Method::POST,
            format!("/api/bookings/{b_resched}/reschedule"),
            Some("CUSTOMER@customer-alice"),
            Some(json!({ "branch": B01, "slotId": slot2_id })),
            Some(json!({ "branch": B01, "slotId": slot_id })),
        ),
        (
            "bookingQrReissue",
            Method::POST,
            format!("/api/bookings/{b_qr}/qr"),
            Some("CUSTOMER@customer-alice"),
            Some(json!({ "branch": B01 })),
            Some(json!({ "branch": B01, "reason": "lost" })),
        ),
    ];

    for (name, method, uri, actor, payload1, payload2) in cases {
        let key = format!("key-idemp-{}", name.to_lowercase());

        // 1. Initial call
        let res1 = call(
            &db,
            actor,
            method.clone(),
            &uri,
            payload1.clone(),
            Some(&key),
        )
        .await;
        assert_eq!(
            res1.status,
            StatusCode::OK,
            "{name}: initial call with Idempotency-Key must succeed 200 OK; got {} {}",
            res1.status,
            res1.text
        );

        // 2. Replay with identical payload
        let res2 = call(&db, actor, method.clone(), &uri, payload1, Some(&key)).await;
        assert_eq!(
            res2.status,
            StatusCode::OK,
            "{name}: replay with same Idempotency-Key must return 200 OK; got {} {}",
            res2.status,
            res2.text
        );

        // 3. Conflict with different payload
        let res3 = call(&db, actor, method, &uri, payload2, Some(&key)).await;
        assert_eq!(
            res3.status,
            StatusCode::CONFLICT,
            "{name}: different payload with same Idempotency-Key must return 409 CONFLICT; got {} {}",
            res3.status,
            res3.text
        );
        assert_eq!(
            res3.body["error"]["code"], "idempotencyConflict",
            "{name}: refusal error code must be idempotencyConflict; got {}",
            res3.text
        );
    }

    db.finish().await;
}
