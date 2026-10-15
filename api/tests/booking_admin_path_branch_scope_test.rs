// Written before the fix by claude/d7aa64. Do not edit, delete or weaken.

//! Booking admin commands must act only on the branch the access gate authorised.
//!
//! The seam (`v3/command.rs` `handle_command`) authorises the caller against the body `branch`
//! (`Policy::authorize(.., named_branch, ..)` → `grant.branch`) and checks `command.locks(..)`
//! against that branch. `slotRuleSet`, `closureAdd` and `closureRemove` keep the default
//! `locks()` (= the granted branch) and then write `BranchId::new(&input.id)` — the path `{id}`.
//! `slotCapacitySet`, `slotClose` and `slotOpen` look the slot's branch up and write it, also
//! without comparing it with the granted branch. So an ADMIN scoped to XC-B01 only
//! (`ScopeRule` allows a branch-scoped ADMIN) who names XC-B01 in the body could write
//! XC-B02's rule, closures and slots.
//!
//! Required: the cross-branch write is refused `403 outOfScope` (`command.rs` `out_of_scope()`)
//! and nothing of XC-B02 changes; the same caller on its own branch still gets 200.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, NaiveDate, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use bk6_common::*;

const B02: &str = "XC-B02";
/// An ADMIN whose only role row is on XC-B01: BranchesManage at XC-B01, not organisation-wide.
const B01_ADMIN: &str = "ADMIN@XC-B01";
const CLOSED_ON: &str = "2026-12-31";

async fn seed_b02(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id, booking_enabled) VALUES ($1, 'Branch 2', $2::uuid, true)",
    )
    .bind(B02)
    .bind(ORG)
    .execute(pool)
    .await
    .expect("insert XC-B02");
    seed_slot_rule(pool, B02, 30, 10, 14).await;
}

async fn seed_closure(pool: &PgPool, branch: &str) {
    sqlx::query(
        "INSERT INTO branch_closures (organization_id, branch_id, closed_on, reason, created_by) \
         VALUES ($1::uuid, $2, $3::date, 'seeded', $4::uuid)",
    )
    .bind(ORG)
    .bind(branch)
    .bind(CLOSED_ON)
    .bind(ADMIN_ID)
    .execute(pool)
    .await
    .expect("seed closure");
}

async fn seed_open_slot(pool: &PgPool, branch: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        pool,
        id,
        branch,
        bangkok_today(),
        now + Duration::hours(1),
        now + Duration::hours(2),
        5,
        0,
        "OPEN",
    )
    .await;
    id
}

async fn rule_of(pool: &PgPool, branch: &str) -> (i32, i32, i32) {
    sqlx::query_as(
        "SELECT slot_minutes, capacity, horizon_days FROM booking_slot_rules WHERE branch_id = $1",
    )
    .bind(branch)
    .fetch_one(pool)
    .await
    .expect("read slot rule")
}

async fn slot_count(pool: &PgPool, branch: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM booking_slots WHERE branch_id = $1")
        .bind(branch)
        .fetch_one(pool)
        .await
        .expect("count slots")
}

async fn closure_reasons(pool: &PgPool, branch: &str) -> Vec<(NaiveDate, String)> {
    sqlx::query_as(
        "SELECT closed_on, reason FROM branch_closures WHERE branch_id = $1 ORDER BY closed_on",
    )
    .bind(branch)
    .fetch_all(pool)
    .await
    .expect("read closures")
}

async fn slot_state(pool: &PgPool, id: Uuid) -> (i32, String) {
    sqlx::query_as("SELECT capacity, status FROM booking_slots WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read slot")
}

fn out_of_scope(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::FORBIDDEN && a.body["error"]["code"] == "outOfScope",
        "{what}: expected 403 outOfScope; got {} {}",
        a.status,
        a.text
    );
}

fn ok(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what}: got {} {}",
        a.status,
        a.text
    );
}

async fn as_b01_admin(db: &TestDb, method: Method, uri: &str, body: Value, key: &str) -> Answer {
    call(db, Some(B01_ADMIN), method, uri, Some(body), Some(key)).await
}

// ── slotRuleSet ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn slot_rule_set_on_a_path_branch_outside_the_grant_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    seed_b02(&db.pool).await;
    let slots_before = slot_count(&db.pool, B02).await;

    let a = as_b01_admin(
        &db,
        Method::PUT,
        &format!("/api/branches/{B02}/booking-slot-rule"),
        json!({ "branch": B01, "slotMinutes": 60, "capacity": 3, "horizonDays": 7 }),
        "k-x-rule",
    )
    .await;

    out_of_scope(&a, "slotRuleSet on XC-B02 by an XC-B01 ADMIN");
    assert_eq!(
        rule_of(&db.pool, B02).await,
        (30, 10, 14),
        "XC-B02's slot rule must be unchanged"
    );
    assert_eq!(
        slot_count(&db.pool, B02).await,
        slots_before,
        "no XC-B02 slots may be generated"
    );
    db.finish().await;
}

// ── closureAdd ─────────────────────────────────────────────────────────────

#[actix_web::test]
async fn closure_add_on_a_path_branch_outside_the_grant_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    seed_b02(&db.pool).await;

    let a = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/branches/{B02}/closures"),
        json!({ "branch": B01, "closedOn": "2026-12-25", "reason": "cross-branch" }),
        "k-x-cadd",
    )
    .await;

    out_of_scope(&a, "closureAdd on XC-B02 by an XC-B01 ADMIN");
    assert!(
        closure_reasons(&db.pool, B02).await.is_empty(),
        "no XC-B02 closure may be written"
    );
    db.finish().await;
}

// ── closureRemove ──────────────────────────────────────────────────────────

#[actix_web::test]
async fn closure_remove_on_a_path_branch_outside_the_grant_is_refused_and_deletes_nothing() {
    let db = TestDb::start().await;
    seed_b02(&db.pool).await;
    seed_closure(&db.pool, B02).await;

    let a = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/branches/{B02}/closures/{CLOSED_ON}/remove"),
        json!({ "branch": B01 }),
        "k-x-crm",
    )
    .await;

    out_of_scope(&a, "closureRemove on XC-B02 by an XC-B01 ADMIN");
    assert_eq!(
        closure_reasons(&db.pool, B02).await.len(),
        1,
        "XC-B02's closure must still be there"
    );
    db.finish().await;
}

// ── slotCapacitySet / slotClose / slotOpen (the slot id names another branch's slot) ──

#[actix_web::test]
async fn slot_commands_on_another_branchs_slot_are_refused_and_change_nothing() {
    let db = TestDb::start().await;
    seed_b02(&db.pool).await;
    let open_slot = seed_open_slot(&db.pool, B02).await;
    let closed_slot = seed_open_slot(&db.pool, B02).await;
    sqlx::query("UPDATE booking_slots SET status = 'CLOSED' WHERE id = $1")
        .bind(closed_slot)
        .execute(&db.pool)
        .await
        .expect("close the seeded slot");

    let cap = as_b01_admin(
        &db,
        Method::PUT,
        &format!("/api/booking-slots/{open_slot}/capacity"),
        json!({ "branch": B01, "capacity": 9 }),
        "k-x-cap",
    )
    .await;
    out_of_scope(&cap, "slotCapacitySet on an XC-B02 slot by an XC-B01 ADMIN");

    let close = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/booking-slots/{open_slot}/close"),
        json!({ "branch": B01 }),
        "k-x-close",
    )
    .await;
    out_of_scope(&close, "slotClose on an XC-B02 slot by an XC-B01 ADMIN");

    let open = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/booking-slots/{closed_slot}/open"),
        json!({ "branch": B01 }),
        "k-x-open",
    )
    .await;
    out_of_scope(&open, "slotOpen on an XC-B02 slot by an XC-B01 ADMIN");

    assert_eq!(
        slot_state(&db.pool, open_slot).await,
        (5, "OPEN".to_string()),
        "the open XC-B02 slot must be unchanged"
    );
    assert_eq!(
        slot_state(&db.pool, closed_slot).await,
        (5, "CLOSED".to_string()),
        "the closed XC-B02 slot must stay closed"
    );
    db.finish().await;
}

// ── control: naming XC-B02 in the body is already refused by the gate ──────

#[actix_web::test]
async fn naming_the_other_branch_in_the_body_is_refused_by_the_gate() {
    let db = TestDb::start().await;
    seed_b02(&db.pool).await;

    let a = as_b01_admin(
        &db,
        Method::PUT,
        &format!("/api/branches/{B02}/booking-slot-rule"),
        json!({ "branch": B02, "slotMinutes": 60, "capacity": 3, "horizonDays": 7 }),
        "k-ctl-rule",
    )
    .await;
    out_of_scope(&a, "slotRuleSet naming XC-B02 in the body");

    let c = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/branches/{B02}/closures"),
        json!({ "branch": B02, "closedOn": "2026-12-25", "reason": "cross-branch" }),
        "k-ctl-cadd",
    )
    .await;
    out_of_scope(&c, "closureAdd naming XC-B02 in the body");

    assert_eq!(rule_of(&db.pool, B02).await, (30, 10, 14));
    assert!(closure_reasons(&db.pool, B02).await.is_empty());
    db.finish().await;
}

// ── keep-pin: the same caller on its own branch still succeeds ──────────────

#[actix_web::test]
async fn the_branch_admin_still_manages_its_own_branch() {
    let db = TestDb::start().await;
    seed_b02(&db.pool).await;
    seed_closure(&db.pool, B01).await;
    let open_slot = seed_open_slot(&db.pool, B01).await;

    let rule = as_b01_admin(
        &db,
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        json!({ "branch": B01, "slotMinutes": 60, "capacity": 3, "horizonDays": 7 }),
        "k-own-rule",
    )
    .await;
    ok(&rule, "slotRuleSet on XC-B01");
    assert_eq!(rule_of(&db.pool, B01).await, (60, 3, 7));

    let add = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/branches/{B01}/closures"),
        json!({ "branch": B01, "closedOn": "2026-12-25", "reason": "own" }),
        "k-own-cadd",
    )
    .await;
    ok(&add, "closureAdd on XC-B01");

    let rm = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/branches/{B01}/closures/{CLOSED_ON}/remove"),
        json!({ "branch": B01 }),
        "k-own-crm",
    )
    .await;
    ok(&rm, "closureRemove on XC-B01");
    let closures = closure_reasons(&db.pool, B01).await;
    assert_eq!(closures.len(), 1, "added one, removed one: {closures:?}");
    assert_eq!(closures[0].1, "own");

    let cap = as_b01_admin(
        &db,
        Method::PUT,
        &format!("/api/booking-slots/{open_slot}/capacity"),
        json!({ "branch": B01, "capacity": 9 }),
        "k-own-cap",
    )
    .await;
    ok(&cap, "slotCapacitySet on an XC-B01 slot");
    let close = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/booking-slots/{open_slot}/close"),
        json!({ "branch": B01 }),
        "k-own-close",
    )
    .await;
    ok(&close, "slotClose on an XC-B01 slot");
    let open = as_b01_admin(
        &db,
        Method::POST,
        &format!("/api/booking-slots/{open_slot}/open"),
        json!({ "branch": B01 }),
        "k-own-open",
    )
    .await;
    ok(&open, "slotOpen on an XC-B01 slot");
    assert_eq!(
        slot_state(&db.pool, open_slot).await,
        (9, "OPEN".to_string())
    );

    assert_eq!(
        rule_of(&db.pool, B02).await,
        (30, 10, 14),
        "XC-B02 untouched"
    );
    db.finish().await;
}

// ── cross-organisation: the strongest org-A grant still cannot reach org B ──

/// A second organisation, with a branch that has its own slot rule and an open slot.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
const ORG_B_BRANCH: &str = "XC-OB1";
/// Organisation-wide ADMIN of `ORG` (`ScopedRole(Admin, None)`): the widest grant org A can hold,
/// so the only thing that can refuse it is the organisation boundary.
const ORG_A_WIDE_ADMIN: &str = "ADMIN@*";

async fn seed_org_b(pool: &PgPool) -> Uuid {
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1::uuid, 'ORGB', 'Asia/Bangkok', now()) ON CONFLICT (id) DO NOTHING",
    )
    .bind(ORG_B)
    .execute(pool)
    .await
    .expect("insert organisation B");
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id, booking_enabled) VALUES ($1, 'Org B branch', $2::uuid, true)",
    )
    .bind(ORG_B_BRANCH)
    .bind(ORG_B)
    .execute(pool)
    .await
    .expect("insert org B branch");
    sqlx::query(
        "INSERT INTO booking_slot_rules (organization_id, branch_id, slot_minutes, capacity, horizon_days, updated_by) \
         VALUES ($1::uuid, $2, 30, 10, 14, $3::uuid)",
    )
    .bind(ORG_B)
    .bind(ORG_B_BRANCH)
    .bind(ADMIN_ID)
    .execute(pool)
    .await
    .expect("seed org B slot rule");
    let slot = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, 5, 0, 'OPEN')",
    )
    .bind(slot)
    .bind(ORG_B)
    .bind(ORG_B_BRANCH)
    .bind(bangkok_today())
    .bind(now + Duration::hours(1))
    .bind(now + Duration::hours(2))
    .execute(pool)
    .await
    .expect("seed org B slot");
    slot
}

#[actix_web::test]
async fn an_org_wide_admin_of_another_organisation_cannot_write_org_b_by_path_or_slot_id() {
    let db = TestDb::start().await;
    let org_b_slot = seed_org_b(&db.pool).await;
    let slots_before = slot_count(&db.pool, ORG_B_BRANCH).await;

    let rule = call(
        &db,
        Some(ORG_A_WIDE_ADMIN),
        Method::PUT,
        &format!("/api/branches/{ORG_B_BRANCH}/booking-slot-rule"),
        Some(json!({ "branch": B01, "slotMinutes": 60, "capacity": 3, "horizonDays": 7 })),
        Some("k-xorg-rule"),
    )
    .await;
    out_of_scope(
        &rule,
        "slotRuleSet on an org-B branch by an org-A org-wide ADMIN",
    );

    let add = call(
        &db,
        Some(ORG_A_WIDE_ADMIN),
        Method::POST,
        &format!("/api/branches/{ORG_B_BRANCH}/closures"),
        Some(json!({ "branch": B01, "closedOn": "2026-12-25", "reason": "cross-org" })),
        Some("k-xorg-cadd"),
    )
    .await;
    out_of_scope(
        &add,
        "closureAdd on an org-B branch by an org-A org-wide ADMIN",
    );

    let cap = call(
        &db,
        Some(ORG_A_WIDE_ADMIN),
        Method::PUT,
        &format!("/api/booking-slots/{org_b_slot}/capacity"),
        Some(json!({ "branch": B01, "capacity": 9 })),
        Some("k-xorg-cap"),
    )
    .await;
    out_of_scope(
        &cap,
        "slotCapacitySet on an org-B slot by an org-A org-wide ADMIN",
    );

    assert_eq!(
        rule_of(&db.pool, ORG_B_BRANCH).await,
        (30, 10, 14),
        "org B's slot rule must be unchanged"
    );
    assert_eq!(
        slot_count(&db.pool, ORG_B_BRANCH).await,
        slots_before,
        "no org-B slots may be generated"
    );
    assert!(
        closure_reasons(&db.pool, ORG_B_BRANCH).await.is_empty(),
        "no org-B closure may be written"
    );
    assert_eq!(
        slot_state(&db.pool, org_b_slot).await,
        (5, "OPEN".to_string()),
        "org B's slot must be unchanged"
    );
    db.finish().await;
}
