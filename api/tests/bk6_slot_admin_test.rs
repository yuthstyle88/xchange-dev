// Written before the implementation by the test writer gemini/pool-5 (controller claude/e92db6). Do not edit, delete or weaken.

//! Specification tests for BK-6 (#1388, docs/design/blueprint/booking-slots.md §4.4, §9.1):
//! slot admin commands: slotRuleSet (`PUT /api/branches/{id}/booking-slot-rule`),
//! closureAdd (`POST /api/branches/{id}/closures`), closureRemove (`POST /api/branches/{id}/closures/{date}/remove`),
//! slotCapacitySet (`PUT /api/booking-slots/{id}/capacity`), slotClose (`POST /api/booking-slots/{id}/close`),
//! slotOpen (`POST /api/booking-slots/{id}/open`), and bookingPolicySet (`PUT /api/booking-policy`).

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use bk6_common::*;

#[actix_web::test]
async fn slot_admin_routes_refuse_without_branches_manage() {
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

    // TELLER has BookingsDesk, not BranchesManage
    let who = Some("TELLER@XC-B01");

    // 1. PUT /api/branches/{id}/booking-slot-rule
    let r1 = call(
        &db,
        who,
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        Some(json!({ "slotMinutes": 30, "capacity": 5, "horizonDays": 14 })),
        Some("k-adm-1"),
    )
    .await;
    policy_refused(&r1, "slotRuleSet by TELLER");

    // 2. POST /api/branches/{id}/closures
    let r2 = call(
        &db,
        who,
        Method::POST,
        &format!("/api/branches/{B01}/closures"),
        Some(json!({ "closedOn": "2026-11-01", "reason": "Holiday" })),
        Some("k-adm-2"),
    )
    .await;
    policy_refused(&r2, "closureAdd by TELLER");

    // 3. POST /api/branches/{id}/closures/{date}/remove
    let r3 = call(
        &db,
        who,
        Method::POST,
        &format!("/api/branches/{B01}/closures/2026-11-01/remove"),
        None,
        Some("k-adm-3"),
    )
    .await;
    policy_refused(&r3, "closureRemove by TELLER");

    // 4. PUT /api/booking-slots/{id}/capacity
    let r4 = call(
        &db,
        who,
        Method::PUT,
        &format!("/api/booking-slots/{slot_id}/capacity"),
        Some(json!({ "capacity": 10 })),
        Some("k-adm-4"),
    )
    .await;
    policy_refused(&r4, "slotCapacitySet by TELLER");

    // 5. POST /api/booking-slots/{id}/close
    let r5 = call(
        &db,
        who,
        Method::POST,
        &format!("/api/booking-slots/{slot_id}/close"),
        None,
        Some("k-adm-5"),
    )
    .await;
    policy_refused(&r5, "slotClose by TELLER");

    // 6. POST /api/booking-slots/{id}/open
    let r6 = call(
        &db,
        who,
        Method::POST,
        &format!("/api/booking-slots/{slot_id}/open"),
        None,
        Some("k-adm-6"),
    )
    .await;
    policy_refused(&r6, "slotOpen by TELLER");

    // 7. PUT /api/booking-policy
    let r7 = call(
        &db,
        who,
        Method::PUT,
        "/api/booking-policy",
        Some(json!({ "cancelCutoffMinutes": 45, "noShowGraceMinutes": 20, "confirmExpiryMinutes": 30 })),
        Some("k-adm-7"),
    )
    .await;
    policy_refused(&r7, "bookingPolicySet by TELLER");

    db.finish().await;
}

#[actix_web::test]
async fn slot_rule_set_updates_rule_and_generates_slots() {
    let db = TestDb::start().await;
    let uri = format!("/api/branches/{B01}/booking-slot-rule");
    let payload = json!({
        "branch": B01,
        "slotMinutes": 30,
        "capacity": 8,
        "horizonDays": 21,
    });

    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::PUT,
        &uri,
        Some(payload),
        Some("k-rule-set-1"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "slotRuleSet by ADMIN must succeed 200; got {} {}",
        res.status,
        res.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn closure_add_and_remove_manages_branch_closures() {
    let db = TestDb::start().await;
    let add_uri = format!("/api/branches/{B01}/closures");
    let payload = json!({
        "branch": B01,
        "closedOn": "2026-12-25",
        "reason": "Christmas Day"
    });

    // 1. Add closure
    let res_add = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &add_uri,
        Some(payload),
        Some("k-clos-add"),
    )
    .await;
    assert_eq!(
        res_add.status,
        StatusCode::OK,
        "closureAdd should succeed 200"
    );

    // 2. Remove closure
    let rm_uri = format!("/api/branches/{B01}/closures/2026-12-25/remove");
    let res_rm = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &rm_uri,
        Some(json!({ "branch": B01 })),
        Some("k-clos-rm"),
    )
    .await;
    assert_eq!(
        res_rm.status,
        StatusCode::OK,
        "closureRemove should succeed 200"
    );

    db.finish().await;
}

#[actix_web::test]
async fn slot_capacity_set_refuses_capacity_below_reserved() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    // Reserved = 3, Capacity = 5
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(1),
        now + Duration::hours(2),
        5,
        3,
        "OPEN",
    )
    .await;

    let uri = format!("/api/booking-slots/{slot_id}/capacity");
    // Attempting to set capacity to 2 (which is < reserved 3)
    let payload = json!({ "branch": B01, "capacity": 2 });

    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::PUT,
        &uri,
        Some(payload),
        Some("k-cap-low"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::BAD_REQUEST,
        "capacity below reserved must be refused with 400; got {} {}",
        res.status,
        res.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn slot_capacity_set_updates_capacity() {
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

    let uri = format!("/api/booking-slots/{slot_id}/capacity");
    let payload = json!({ "branch": B01, "capacity": 10 });

    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::PUT,
        &uri,
        Some(payload),
        Some("k-cap-up"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "slotCapacitySet should succeed 200"
    );

    db.finish().await;
}

#[actix_web::test]
async fn slot_close_and_open_toggles_slot_status() {
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

    // 1. Close slot
    let close_uri = format!("/api/booking-slots/{slot_id}/close");
    let res_close = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &close_uri,
        Some(json!({ "branch": B01 })),
        Some("k-s-close"),
    )
    .await;
    assert_eq!(
        res_close.status,
        StatusCode::OK,
        "slotClose should succeed 200"
    );

    // 2. Open slot
    let open_uri = format!("/api/booking-slots/{slot_id}/open");
    let res_open = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &open_uri,
        Some(json!({ "branch": B01 })),
        Some("k-s-open"),
    )
    .await;
    assert_eq!(
        res_open.status,
        StatusCode::OK,
        "slotOpen should succeed 200"
    );

    db.finish().await;
}

#[actix_web::test]
async fn booking_policy_set_updates_policy() {
    let db = TestDb::start().await;
    let uri = "/api/booking-policy";
    let payload = json!({
        "branch": B01,
        "cancelCutoffMinutes": 45,
        "noShowGraceMinutes": 20,
        "confirmExpiryMinutes": 30,
    });

    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::PUT,
        uri,
        Some(payload),
        Some("k-pol-set"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "bookingPolicySet should succeed 200"
    );

    db.finish().await;
}

#[actix_web::test]
async fn slot_admin_idempotency_replay_and_conflict() {
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

    // Seed a closure for closureRemove
    sqlx::query(
        "INSERT INTO branch_closures (organization_id, branch_id, closed_on, reason, created_by) \
         VALUES ($1::uuid, $2, '2026-12-31', 'New Year Eve', $3::uuid) \
         ON CONFLICT (branch_id, closed_on) DO NOTHING",
    )
    .bind(ORG)
    .bind(B01)
    .bind(ADMIN_ID)
    .execute(&db.pool)
    .await
    .expect("seed branch closure");

    let cases = vec![
        (
            "slotRuleSet",
            Method::PUT,
            format!("/api/branches/{B01}/booking-slot-rule"),
            Some(json!({ "branch": B01, "slotMinutes": 30, "capacity": 10, "horizonDays": 14 })),
            Some(json!({ "branch": B01, "slotMinutes": 60, "capacity": 10, "horizonDays": 14 })),
        ),
        (
            "closureAdd",
            Method::POST,
            format!("/api/branches/{B01}/closures"),
            Some(json!({ "branch": B01, "closedOn": "2026-12-25", "reason": "Christmas Day" })),
            Some(json!({ "branch": B01, "closedOn": "2026-12-25", "reason": "Renovation" })),
        ),
        (
            "closureRemove",
            Method::POST,
            format!("/api/branches/{B01}/closures/2026-12-31/remove"),
            Some(json!({ "branch": B01 })),
            Some(json!({ "branch": B01, "reason": "reopened" })),
        ),
        (
            "slotCapacitySet",
            Method::PUT,
            format!("/api/booking-slots/{slot_id}/capacity"),
            Some(json!({ "branch": B01, "capacity": 8 })),
            Some(json!({ "branch": B01, "capacity": 12 })),
        ),
        (
            "slotClose",
            Method::POST,
            format!("/api/booking-slots/{slot_id}/close"),
            Some(json!({ "branch": B01 })),
            Some(json!({ "branch": B01, "reason": "maintenance" })),
        ),
        (
            "slotOpen",
            Method::POST,
            format!("/api/booking-slots/{slot_id}/open"),
            Some(json!({ "branch": B01 })),
            Some(json!({ "branch": B01, "reason": "reopened" })),
        ),
        (
            "bookingPolicySet",
            Method::PUT,
            "/api/booking-policy".to_string(),
            Some(
                json!({ "branch": B01, "cancelCutoffMinutes": 50, "noShowGraceMinutes": 10, "confirmExpiryMinutes": 40 }),
            ),
            Some(
                json!({ "branch": B01, "cancelCutoffMinutes": 90, "noShowGraceMinutes": 10, "confirmExpiryMinutes": 40 }),
            ),
        ),
    ];

    for (name, method, uri, payload1, payload2) in cases {
        let key = format!("key-idemp-admin-{}", name.to_lowercase());

        // 1. Initial call
        let res1 = call(
            &db,
            Some("ADMIN@*"),
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
        let res2 = call(
            &db,
            Some("ADMIN@*"),
            method.clone(),
            &uri,
            payload1,
            Some(&key),
        )
        .await;
        assert_eq!(
            res2.status,
            StatusCode::OK,
            "{name}: replay with same Idempotency-Key must return 200 OK; got {} {}",
            res2.status,
            res2.text
        );

        // 3. Conflict with different payload
        let res3 = call(&db, Some("ADMIN@*"), method, &uri, payload2, Some(&key)).await;
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
