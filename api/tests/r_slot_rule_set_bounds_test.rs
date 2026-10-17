// Written before the implementation by claude/studio-claude-2 (BE controller claude/56a74fcd). Do not edit, delete or weaken.

//! R-ticket of #2054 for `slotRuleSet` (`PUT /api/branches/{id}/booking-slot-rule`,
//! `commands/bookings/slot_rule.rs`), design `docs/design/blueprint/bounded-inputs.md` B-3, B-7 (`IntBound`).
//!
//! The columns' CHECKs (`migrations/20261205131000_booking_slots.sql:36-38`): `slot_minutes BETWEEN 5 AND 240`,
//! `capacity > 0`, `horizon_days BETWEEN 1 AND 90`. A value outside them is **400 `validationFailed`** naming the
//! field, and the branch's rule row is unchanged. Both sides of every bound are pinned.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use serde_json::json;
use uuid::Uuid;

use bk6_common::*;

/// The rule `TestDb::start` seeds for XC-B01 (`bk6_common/mod.rs`, `seed_slot_rule(&pool, B01, 30, 10, 14)`).
const SEEDED: (i32, i32, i32) = (30, 10, 14);

async fn set_rule(db: &TestDb, slot_minutes: i64, capacity: i64, horizon_days: i64) -> Answer {
    call(
        db,
        Some("ADMIN@*"),
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        Some(json!({
            "branch": B01,
            "slotMinutes": slot_minutes,
            "capacity": capacity,
            "horizonDays": horizon_days,
        })),
        Some(&format!("k-r-rule-{}", Uuid::new_v4())),
    )
    .await
}

async fn rule_of(db: &TestDb) -> (i32, i32, i32) {
    sqlx::query_as(
        "SELECT slot_minutes, capacity, horizon_days FROM booking_slot_rules WHERE branch_id = $1",
    )
    .bind(B01)
    .fetch_one(&db.pool)
    .await
    .expect("read the slot rule")
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

async fn refused_on(db: &TestDb, a: &Answer, field: &str, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && a.body["error"]["code"] == "validationFailed"
            && a.body["error"]["field"] == field,
        "{what}: expected 400 validationFailed on `{field}`; got {} {}",
        a.status,
        a.text
    );
    assert_eq!(
        rule_of(db).await,
        SEEDED,
        "{what}: the rule row is unchanged"
    );
}

#[actix_web::test]
async fn slot_minutes_accepts_5_and_240_and_refuses_4_and_241() {
    let db = TestDb::start().await;
    accepted(&set_rule(&db, 5, 10, 14).await, "slotMinutes 5");
    accepted(&set_rule(&db, 240, 10, 14).await, "slotMinutes 240");
    seed_slot_rule(&db.pool, B01, SEEDED.0, SEEDED.1, SEEDED.2).await;
    refused_on(
        &db,
        &set_rule(&db, 4, 10, 14).await,
        "slotMinutes",
        "slotMinutes 4",
    )
    .await;
    refused_on(
        &db,
        &set_rule(&db, 241, 10, 14).await,
        "slotMinutes",
        "slotMinutes 241",
    )
    .await;
    db.finish().await;
}

#[actix_web::test]
async fn capacity_accepts_1_and_refuses_0_and_negative() {
    let db = TestDb::start().await;
    accepted(&set_rule(&db, 30, 1, 14).await, "capacity 1");
    accepted(
        &set_rule(&db, 30, i32::MAX as i64, 14).await,
        "capacity i32::MAX",
    );
    seed_slot_rule(&db.pool, B01, SEEDED.0, SEEDED.1, SEEDED.2).await;
    refused_on(
        &db,
        &set_rule(&db, 30, 0, 14).await,
        "capacity",
        "capacity 0",
    )
    .await;
    refused_on(
        &db,
        &set_rule(&db, 30, -1, 14).await,
        "capacity",
        "capacity -1",
    )
    .await;
    db.finish().await;
}

#[actix_web::test]
async fn horizon_days_accepts_1_and_90_and_refuses_0_and_91() {
    let db = TestDb::start().await;
    accepted(&set_rule(&db, 30, 10, 1).await, "horizonDays 1");
    accepted(&set_rule(&db, 30, 10, 90).await, "horizonDays 90");
    seed_slot_rule(&db.pool, B01, SEEDED.0, SEEDED.1, SEEDED.2).await;
    refused_on(
        &db,
        &set_rule(&db, 30, 10, 0).await,
        "horizonDays",
        "horizonDays 0",
    )
    .await;
    refused_on(
        &db,
        &set_rule(&db, 30, 10, 91).await,
        "horizonDays",
        "horizonDays 91",
    )
    .await;
    db.finish().await;
}
