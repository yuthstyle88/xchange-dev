// Written before the implementation by claude/d7aa64 (BE controller claude/7ca80c). Do not edit, delete or weaken.

//! #1679's code step: the v3 booking commands run on the shared `CommandEndpoint`
//! (`docs/design/blueprint/booking-slots.md` §4.5, merged as PR 1890 `e4d5f5d6`). The behaviour the
//! booking copy (`handle_booking_command`) lacks today, pinned so the move cannot skip it:
//!
//! 1. the same-origin guard (§4.5 row 1)                       — red on main
//! 2. a refused command releases its key (row 2)               — red on main
//! 3. a malformed input takes no key (row 5)                   — red on main
//! 4. the named branch must be the booking's (ruling on #1679) — red on main
//! 5. a replayed QR reissue carries no QR token (row 6)        — a pin, green on main
//! 6. the kiosk check-in names its branch as `branch` (Q1)     — red on main
//!
//! Row 3 (the branch lock) has no test, on purpose: a lock is only observable under a race, and the move
//! takes it through `BranchTx` like every other command.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, test as atest, web};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::SessionReader;

use bk6_common::*;

const B02: &str = "XC-B02";

/// A request **without** `x-xchange-request` and `Origin`, the headers `call` always adds: what a cross-site
/// form post looks like.
async fn call_without_origin(db: &TestDb, who: &str, uri: &str, body: Value, key: &str) -> Answer {
    let state = AppState::new(Arc::new(test_common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn status_of(db: &TestDb, booking_id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
        .bind(booking_id)
        .fetch_one(&db.pool)
        .await
        .expect("the booking's status")
}

/// A slot in two hours on B01 and one booking of Alice's in it.
async fn alice_booking(db: &TestDb, booking_id: &str, booking_no: &str, status: &str, qr: &str) {
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
    seed_booking(
        &db.pool,
        booking_id,
        booking_no,
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        status,
        &Sha256::digest(qr.as_bytes()),
    )
    .await;
}

// 1.
#[actix_web::test]
async fn a_booking_command_without_the_same_origin_headers_is_refused() {
    let db = TestDb::start().await;
    let id = "BK-shared-origin";
    alice_booking(
        &db,
        id,
        "B261015-0101",
        "PENDING_CUSTOMER_CONFIRMATION",
        "qr-origin",
    )
    .await;

    let res = call_without_origin(
        &db,
        "CUSTOMER@customer-alice",
        &format!("/api/bookings/{id}/confirm"),
        json!({ "branch": B01 }),
        "k-shared-origin",
    )
    .await;
    refused_with(
        &res,
        "originRejected",
        "a booking command without x-xchange-request/Origin",
    );
    assert_eq!(
        status_of(&db, id).await,
        "PENDING_CUSTOMER_CONFIRMATION",
        "a refused command changes nothing"
    );

    db.finish().await;
}

// 2.
#[actix_web::test]
async fn a_refused_command_releases_its_key_so_the_same_retry_is_refused_again_not_a_conflict() {
    let db = TestDb::start().await;
    let id = "BK-shared-release";
    alice_booking(&db, id, "B261015-0102", "CONFIRMED", "qr-release").await;
    let uri = format!("/api/bookings/{id}/cancel");

    for attempt in ["first", "second (same key, same body, within 60 s)"] {
        let res = call(
            &db,
            Some("TELLER@XC-B01"),
            Method::POST,
            &uri,
            Some(json!({ "branch": B01 })),
            Some("k-shared-release"),
        )
        .await;
        refused_with(&res, "validationFailed", attempt);
    }
    assert_eq!(status_of(&db, id).await, "CONFIRMED");

    db.finish().await;
}

// 3.
#[actix_web::test]
async fn a_malformed_input_takes_no_key_so_the_corrected_request_with_that_key_runs() {
    let db = TestDb::start().await;
    let id = "BK-shared-typed";
    alice_booking(&db, id, "B261015-0103", "CONFIRMED", "qr-typed").await;
    let uri = format!("/api/bookings/{id}/cancel");

    let malformed = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01, "reason": 5 })),
        Some("k-shared-typed"),
    )
    .await;
    assert!(
        malformed.status.is_client_error()
            && malformed.body["error"]["code"] != "idempotencyConflict",
        "a reason that is not text is refused as a bad request: {} {}",
        malformed.status,
        malformed.text
    );

    let corrected = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01, "reason": "customer asked by phone" })),
        Some("k-shared-typed"),
    )
    .await;
    assert_eq!(
        corrected.status,
        StatusCode::OK,
        "the key was never taken by the malformed request: {}",
        corrected.text
    );
    assert_eq!(status_of(&db, id).await, "CANCELLED");

    db.finish().await;
}

// 4.
#[actix_web::test]
async fn a_booking_command_that_names_another_branch_is_out_of_scope() {
    let db = TestDb::start().await;
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id, booking_enabled) VALUES ($1, 'Branch 2', $2::uuid, true)",
    )
    .bind(B02)
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("a second branch of the same organisation");
    let id = "BK-shared-branch";
    alice_booking(
        &db,
        id,
        "B261015-0104",
        "PENDING_CUSTOMER_CONFIRMATION",
        "qr-branch",
    )
    .await;

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{id}/confirm"),
        Some(json!({ "branch": B02 })),
        Some("k-shared-branch"),
    )
    .await;
    refused_with(
        &res,
        "outOfScope",
        "a B01 booking confirmed while naming B02",
    );
    assert_eq!(status_of(&db, id).await, "PENDING_CUSTOMER_CONFIRMATION");

    db.finish().await;
}

// 5.
#[actix_web::test]
async fn a_replayed_qr_reissue_answers_without_the_qr_token() {
    let db = TestDb::start().await;
    // QR reissue parses the booking id as a UUID (`qr.rs`).
    let id = Uuid::new_v4().to_string();
    let id = id.as_str();
    alice_booking(&db, id, "B261015-0105", "CONFIRMED", "qr-old").await;
    let uri = format!("/api/bookings/{id}/qr");

    let first = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-shared-qr"),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert!(
        first.body["qrToken"].as_str().is_some(),
        "the first answer carries the new token: {}",
        first.text
    );

    let replay = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some("k-shared-qr"),
    )
    .await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.text);
    assert!(
        replay.body["qrToken"].is_null(),
        "a replay must not hand out the live QR token again: {}",
        replay.text
    );

    db.finish().await;
}

// 6.
#[actix_web::test]
async fn the_kiosk_check_in_names_its_branch_as_branch() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;
    let id = "BK-shared-kiosk";
    let token = "kiosk-shared-token";
    seed_booking(
        &db.pool,
        id,
        "B261015-0106",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &Sha256::digest(token.as_bytes()),
    )
    .await;

    let res = call(
        &db,
        None,
        Method::POST,
        "/api/kiosk/bookings/check-in",
        Some(json!({ "branch": B01, "qrToken": token })),
        Some("k-shared-kiosk"),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "a public command with no signed-in user, its branch named `branch`: {}",
        res.text
    );
    assert_eq!(status_of(&db, id).await, "CHECKED_IN");

    db.finish().await;
}
