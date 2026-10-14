// Written before the implementation by the test writer gemini/pool-5 (controller claude/e92db6). Do not edit, delete or weaken.

//! Specification tests for BK-6 (#1388, docs/design/blueprint/booking-slots.md §4.4, §9.1):
//! v2 envelope moved to `POST /api/v2/bookings`, answering as before for owner,
//! refusing without BookingsAdmin, and verifying `POST /api/bookings` no longer serves the v2 envelope.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use serde_json::json;

use bk6_common::*;

#[actix_web::test]
async fn v2_envelope_answers_on_v2_bookings_for_owner() {
    let db = TestDb::start().await;
    let uri = "/api/v2/bookings";
    let payload = json!({
        "action": "list",
        "branchId": B01,
    });

    let res = call(&db, Some("OWNER@*"), Method::POST, uri, Some(payload), None).await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "v2 envelope on /api/v2/bookings must answer 200 OK for owner; got {} {}",
        res.status,
        res.text
    );

    // v2 list response contains bookings array
    assert!(
        res.body["bookings"].is_array() || res.body["data"]["bookings"].is_array(),
        "v2 envelope list must return bookings array; got {}",
        res.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn v2_envelope_refuses_without_bookings_admin_or_owner() {
    let db = TestDb::start().await;
    let uri = "/api/v2/bookings";
    let payload = json!({
        "action": "list",
        "branchId": B01,
    });

    // 1. Unauthenticated -> 401
    let res_anon = call(&db, None, Method::POST, uri, Some(payload.clone()), None).await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // 2. TELLER (no BookingsAdmin) -> refused (403 ownerOnly or 400 roleNotAllowed)
    let res_teller = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        uri,
        Some(payload.clone()),
        None,
    )
    .await;
    assert!(
        res_teller.status == StatusCode::FORBIDDEN || res_teller.status == StatusCode::BAD_REQUEST,
        "v2 envelope must refuse caller without BookingsAdmin; got {} {}",
        res_teller.status,
        res_teller.text
    );

    // 3. Customer -> refused
    let res_cust = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        uri,
        Some(payload),
        None,
    )
    .await;
    assert!(
        res_cust.status == StatusCode::FORBIDDEN || res_cust.status == StatusCode::BAD_REQUEST,
        "v2 envelope must refuse customer; got {} {}",
        res_cust.status,
        res_cust.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn v2_envelope_no_longer_answers_on_old_api_bookings_route() {
    let db = TestDb::start().await;
    let old_uri = "/api/bookings";
    let v2_list_payload = json!({
        "action": "list",
        "branchId": B01,
    });

    // When the owner sends a v2 envelope payload to the old /api/bookings route,
    // it must NOT succeed as a v2 list response, because /api/bookings is now
    // the v3 bookingCreate command endpoint (which expects slotId, shopSide, etc.).
    let res = call(
        &db,
        Some("OWNER@*"),
        Method::POST,
        old_uri,
        Some(v2_list_payload),
        Some("k-old-v2"),
    )
    .await;

    // Must not return v2 list answer with bookings array
    assert!(
        res.body["bookings"].is_null(),
        "old /api/bookings route must no longer serve the v2 list action; got {}",
        res.text
    );

    db.finish().await;
}
