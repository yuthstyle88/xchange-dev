//! Written before the implementation (#3477, up-front tests by claude/studio-claude-3 on the
//! xchange BE controller claude/a90ab16c's order). Do not edit, delete or weaken.
//!
//! The booking views show each booking's verification readiness from `Readiness::of` (VF-6,
//! `docs/design/blueprint/verification.md:97` V12; `booking-slots.md:240-241`): `my_bookings` (C03)
//! for the caller's own bookings, `desk_bookings` (B02) per booking's customer. The values are
//! `my_verification`'s five (`"NONE" | "PENDING" | "NEEDS_UPDATE" | "VERIFIED" | "EXPIRED"`), and
//! `"unknown"` is no longer produced. One test per row of the ticket's §9.
//!
//! The EXPIRED row seeds a case whose status is `EXPIRED` with `expires_at` in the past, as
//! `readiness_test.rs` does: `Readiness::of`'s rules are VF-6's and not this ticket's.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use bk6_common::*;

const ALICE: &str = "CUSTOMER@customer-alice";
const TELLER: &str = "TELLER@XC-B01";

/// A verification case for `customer`, made `minutes_ago`, in `status`, seeded as
/// `xc-verification/tests/readiness_test.rs` seeds that status.
async fn case(pool: &PgPool, customer: &str, status: &str, minutes_ago: i64) {
    let created = Utc::now() - Duration::minutes(minutes_ago);
    let expires = match status {
        "VERIFIED" => Some(Utc::now() + Duration::days(90)),
        "EXPIRED" => Some(Utc::now() - Duration::days(5)),
        _ => None,
    };
    let decided = matches!(status, "VERIFIED" | "EXPIRED" | "NEEDS_UPDATE");
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2::uuid, $3, 'STAFF', $4, 1, \
                 CASE WHEN $4 = 'DRAFT' THEN NULL ELSE $5 END, \
                 CASE WHEN $6 THEN $5 END, $7, \
                 CASE WHEN $6 THEN $8::uuid END, \
                 CASE WHEN $4 = 'NEEDS_UPDATE' THEN 'DETAILS_MISMATCH' END, \
                 CASE WHEN $6 THEN 2 ELSE 1 END, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG)
    .bind(customer)
    .bind(status)
    .bind(created)
    .bind(decided)
    .bind(expires)
    .bind(MANAGER_ID)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("seed a {status} case for {customer}: {e}"));
}

/// A slot today in `B01` and a CONFIRMED booking of `customer` in it.
async fn booking(pool: &PgPool, id: &str, booking_no: &str, customer: &str) {
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
    let qr_hash = Sha256::digest(id.as_bytes());
    seed_booking(
        pool,
        id,
        booking_no,
        customer,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;
}

/// The object in the answer whose `id` is `id`, wherever the view puts its list.
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

async fn readiness_in(db: &TestDb, who: &str, view: &str, id: &str) -> String {
    let uri = format!("/api/v3/views/{view}?branch={B01}");
    let res = call(db, Some(who), Method::GET, &uri, None, None).await;
    assert_eq!(res.status, StatusCode::OK, "{view}: {}", res.text);
    let row = row_of(&res.body, id).unwrap_or_else(|| panic!("{view} lists {id}: {}", res.text));
    row["readiness"]
        .as_str()
        .unwrap_or_else(|| panic!("{view}: `readiness` is text: {}", res.text))
        .to_string()
}

/// One DB, one booking of Alice, Alice's cases as given; what `my_bookings` and `desk_bookings`
/// say for it.
async fn both_views_with(cases: &[(&str, i64)]) -> (String, String) {
    let db = TestDb::start().await;
    for (status, minutes_ago) in cases {
        case(&db.pool, CUSTOMER_ALICE_CUST_ID, status, *minutes_ago).await;
    }
    let id = "BK-rd-alice-1";
    booking(&db.pool, id, "B261019-0101", CUSTOMER_ALICE_CUST_ID).await;
    let mine = readiness_in(&db, ALICE, "my_bookings", id).await;
    let desk = readiness_in(&db, TELLER, "desk_bookings", id).await;
    db.finish().await;
    (mine, desk)
}

/// §9 row 1: no case, or only a DRAFT case: `"NONE"` (VF-6 ruling, `readiness_test.rs:308`).
#[actix_web::test]
async fn no_case_or_only_a_draft_reads_none() {
    let none = both_views_with(&[]).await;
    assert_eq!(
        none,
        ("NONE".into(), "NONE".into()),
        "no case (my_bookings, desk_bookings)"
    );
    let draft = both_views_with(&[("DRAFT", 1)]).await;
    assert_eq!(draft, ("NONE".into(), "NONE".into()), "only a DRAFT case");
}

/// §9 row 2: the newest case awaits review: `"PENDING"`.
#[actix_web::test]
async fn a_case_awaiting_review_reads_pending() {
    for status in ["SUBMITTED", "UNDER_REVIEW"] {
        let got = both_views_with(&[(status, 1)]).await;
        assert_eq!(
            got,
            ("PENDING".into(), "PENDING".into()),
            "newest case {status}"
        );
    }
}

/// §9 row 3: the newest case is VERIFIED, before `expires_at`: `"VERIFIED"`; an older case
/// does not decide it.
#[actix_web::test]
async fn a_verified_case_reads_verified() {
    let got = both_views_with(&[("NEEDS_UPDATE", 60), ("VERIFIED", 1)]).await;
    assert_eq!(got, ("VERIFIED".into(), "VERIFIED".into()));
}

/// §9 row 4: a verified case past `expires_at` (status EXPIRED): `"EXPIRED"`.
#[actix_web::test]
async fn an_expired_case_reads_expired() {
    let got = both_views_with(&[("EXPIRED", 1)]).await;
    assert_eq!(got, ("EXPIRED".into(), "EXPIRED".into()));
}

/// §9 row 5: an update was requested: `"NEEDS_UPDATE"`.
#[actix_web::test]
async fn an_update_requested_reads_needs_update() {
    let got = both_views_with(&[("NEEDS_UPDATE", 1)]).await;
    assert_eq!(got, ("NEEDS_UPDATE".into(), "NEEDS_UPDATE".into()));
}

/// §9 row 6: a `desk_bookings` page with two bookings of one customer and one of another: each
/// row shows its own customer's value.
#[actix_web::test]
async fn each_desk_row_shows_its_own_customers_readiness() {
    let db = TestDb::start().await;
    case(&db.pool, CUSTOMER_ALICE_CUST_ID, "VERIFIED", 1).await;
    case(&db.pool, CUSTOMER_BOB_CUST_ID, "SUBMITTED", 1).await;
    booking(
        &db.pool,
        "BK-rd-alice-1",
        "B261019-0201",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;
    booking(
        &db.pool,
        "BK-rd-alice-2",
        "B261019-0202",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;
    booking(
        &db.pool,
        "BK-rd-bob-1",
        "B261019-0203",
        CUSTOMER_BOB_CUST_ID,
    )
    .await;

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some(TELLER), Method::GET, &uri, None, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    let of = |id: &str| {
        row_of(&res.body, id)
            .and_then(|row| row["readiness"].as_str())
            .unwrap_or_else(|| panic!("desk_bookings lists {id} with a readiness: {}", res.text))
            .to_string()
    };
    assert_eq!(
        (of("BK-rd-alice-1"), of("BK-rd-alice-2"), of("BK-rd-bob-1")),
        ("VERIFIED".into(), "VERIFIED".into(), "PENDING".into()),
        "{}",
        res.text
    );
    assert!(
        !res.text.contains("\"unknown\""),
        "no row reads unknown: {}",
        res.text
    );
    db.finish().await;
}
