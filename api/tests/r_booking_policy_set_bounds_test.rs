// Written before the implementation by claude/studio-claude-2 (BE controller claude/56a74fcd). Do not edit, delete or weaken.

//! R-ticket of #2054 for `bookingPolicySet` (`PUT /api/booking-policy`, `commands/bookings/policy.rs`), design
//! `docs/design/blueprint/bounded-inputs.md` B-3, B-7 (`IntBound`).
//!
//! The columns' CHECKs (`migrations/20261205131000_booking_slots.sql:25-27`): `cancel_cutoff_minutes >= 0`,
//! `no_show_grace_minutes >= 0`, `confirm_expiry_minutes > 0`. A value outside them is **400
//! `validationFailed`** naming the field, and the organisation's policy row is unchanged. Both sides are pinned.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use serde_json::json;
use uuid::Uuid;

use bk6_common::*;

type PolicyRow = (i32, i32, i32);

async fn policy_of(db: &TestDb) -> PolicyRow {
    sqlx::query_as(
        "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
         FROM booking_policies WHERE organization_id = $1::uuid",
    )
    .bind(ORG)
    .fetch_one(&db.pool)
    .await
    .expect("read the booking policy")
}

async fn set_policy(db: &TestDb, cutoff: i64, grace: i64, expiry: i64) -> Answer {
    call(
        db,
        Some("ADMIN@*"),
        Method::PUT,
        "/api/booking-policy",
        Some(json!({
            "cancelCutoffMinutes": cutoff,
            "noShowGraceMinutes": grace,
            "confirmExpiryMinutes": expiry,
        })),
        Some(&format!("k-r-pol-{}", Uuid::new_v4())),
    )
    .await
}

fn accepted(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what}: expected 200; got {} {}",
        a.status,
        a.text
    );
}

async fn refused_on(db: &TestDb, before: PolicyRow, a: &Answer, field: &str, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && a.body["error"]["code"] == "validationFailed"
            && a.body["error"]["field"] == field,
        "{what}: expected 400 validationFailed on `{field}`; got {} {}",
        a.status,
        a.text
    );
    assert_eq!(
        policy_of(db).await,
        before,
        "{what}: the policy row is unchanged"
    );
}

#[actix_web::test]
async fn cancel_cutoff_accepts_0_and_refuses_minus_1() {
    let db = TestDb::start().await;
    accepted(&set_policy(&db, 0, 20, 30).await, "cancelCutoffMinutes 0");
    accepted(
        &set_policy(&db, 129_600, 20, 30).await,
        "cancelCutoffMinutes i32::MAX",
    );
    let before = policy_of(&db).await;
    let a = set_policy(&db, -1, 20, 30).await;
    refused_on(
        &db,
        before,
        &a,
        "cancelCutoffMinutes",
        "cancelCutoffMinutes -1",
    )
    .await;
    db.finish().await;
}

#[actix_web::test]
async fn no_show_grace_accepts_0_and_refuses_minus_1() {
    let db = TestDb::start().await;
    accepted(&set_policy(&db, 45, 0, 30).await, "noShowGraceMinutes 0");
    let before = policy_of(&db).await;
    let a = set_policy(&db, 45, -1, 30).await;
    refused_on(
        &db,
        before,
        &a,
        "noShowGraceMinutes",
        "noShowGraceMinutes -1",
    )
    .await;
    db.finish().await;
}

#[actix_web::test]
async fn confirm_expiry_accepts_1_and_refuses_0() {
    let db = TestDb::start().await;
    accepted(&set_policy(&db, 45, 20, 1).await, "confirmExpiryMinutes 1");
    let before = policy_of(&db).await;
    let a = set_policy(&db, 45, 20, 0).await;
    refused_on(
        &db,
        before,
        &a,
        "confirmExpiryMinutes",
        "confirmExpiryMinutes 0",
    )
    .await;
    let a = set_policy(&db, 45, 20, -5).await;
    refused_on(
        &db,
        before,
        &a,
        "confirmExpiryMinutes",
        "confirmExpiryMinutes -5",
    )
    .await;
    db.finish().await;
}
