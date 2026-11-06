//! Tests for F36/F37a (#6341): `/staff` floor view answers `NOT_OPENED` before a day opens
//! and lists the branch's `openShifts` for a caller holding `CashManage`.

#[allow(unused_imports)]
mod save_common;

use chrono::{DateTime, Utc};
use save_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor, Role};
use xchange::application::staff_floor::StaffFloorReader;
use xchange::presentation::test_callers;

async fn staff_actor(shop: &Shop, name: &str, role: Role) -> Actor {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT organization_id, $2, 'ACTIVE', 'test' FROM branches WHERE id = $1 RETURNING id::text",
    )
    .bind(MAIN)
    .bind(name)
    .fetch_one(&shop.pool)
    .await
    .expect("insert a user");
    test_callers::staff(&id, &[role], branch(MAIN))
}

async fn set_ops(shop: &Shop, phase: &str, cutoff: Option<Value>) {
    sqlx::query(
        "INSERT INTO ops_state (branch_id, phase, cutoff) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (branch_id) DO UPDATE \
         SET phase = EXCLUDED.phase, cutoff = EXCLUDED.cutoff",
    )
    .bind(MAIN)
    .bind(phase)
    .bind(cutoff)
    .execute(&shop.pool)
    .await
    .expect("upsert ops_state");
}

async fn seed_open_shift(shop: &Shop, who: &Actor, opened_at: &str) -> String {
    let id = format!("SH-{}", unique("f36"));
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id, \
             template_id, template_name, business_date) \
         SELECT $1, $2, 'OPEN', $3, 'test', '', $4, $5::uuid, t.id, t.name, DATE '2026-11-05' \
           FROM shift_templates t WHERE t.branch_id = $2 AND t.retired_at IS NULL ORDER BY t.position ASC LIMIT 1",
    )
    .bind(&id)
    .bind(MAIN)
    .bind(instant(opened_at))
    .bind(unique("f36-open").replace('_', "-"))
    .bind(&who.user_id)
    .execute(&shop.pool)
    .await
    .expect("seed an OPEN shift");
    id
}

async fn floor_at(shop: &Shop, who: &Actor, now: &str) -> Value {
    let floor = StaffFloorReader::new(shop.pool.clone())
        .floor_at(who, &branch(MAIN), instant(now))
        .await
        .expect("the floor");
    serde_json::to_value(&floor).expect("the floor serialises")
}

fn cutoff_json(at: &str) -> Value {
    let ms = DateTime::parse_from_rfc3339(at)
        .expect("valid rfc3339")
        .with_timezone(&Utc)
        .timestamp_millis();
    json!({ "at": ms, "by": "bm", "shiftId": "s-1", "pending": [] })
}

/// Row 1: no `ops_state` row, never closed -> `phase` is `NOT_OPENED`, `openShifts` is `[]` for manager and teller.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f36_no_ops_state_row_reads_not_opened_with_empty_open_shifts() {
    let shop = Shop::start().await;
    sqlx::query("DELETE FROM ops_state WHERE branch_id = $1")
        .bind(MAIN)
        .execute(&shop.pool)
        .await
        .expect("clear ops_state");
    let manager = staff_actor(&shop, "Manager", Role::BranchManager).await;
    let teller = staff_actor(&shop, "Teller", Role::Teller).await;
    let now = "2026-11-05T05:00:00Z";

    let mgr_floor = floor_at(&shop, &manager, now).await;
    assert_eq!(mgr_floor["phase"], json!("NOT_OPENED"), "{mgr_floor}");
    assert_eq!(mgr_floor["openShifts"], json!([]), "{mgr_floor}");

    let teller_floor = floor_at(&shop, &teller, now).await;
    assert_eq!(teller_floor["phase"], json!("NOT_OPENED"), "{teller_floor}");
    assert_eq!(teller_floor["openShifts"], json!([]), "{teller_floor}");

    shop.finish().await;
}

/// Row 2: stored `CLOSED` (branch has never opened) -> `phase` is `NOT_OPENED`, `openShifts` is `[]` for manager and teller.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f36_stored_closed_reads_not_opened_with_empty_open_shifts() {
    let shop = Shop::start().await;
    set_ops(&shop, "CLOSED", None).await;
    let manager = staff_actor(&shop, "Manager", Role::BranchManager).await;
    let teller = staff_actor(&shop, "Teller", Role::Teller).await;
    let now = "2026-11-05T05:00:00Z";

    let mgr_floor = floor_at(&shop, &manager, now).await;
    assert_eq!(mgr_floor["phase"], json!("NOT_OPENED"), "{mgr_floor}");
    assert_eq!(mgr_floor["openShifts"], json!([]), "{mgr_floor}");

    let teller_floor = floor_at(&shop, &teller, now).await;
    assert_eq!(teller_floor["phase"], json!("NOT_OPENED"), "{teller_floor}");
    assert_eq!(teller_floor["openShifts"], json!([]), "{teller_floor}");

    shop.finish().await;
}

/// Row 3: stored `CUT_OFF`, 0 open shifts, `business_date_of(cutoff.at) == business_date_of(now)` -> `phase` is `CLOSED`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f36_stored_cutoff_with_no_open_shifts_and_todays_cutoff_reads_closed() {
    let shop = Shop::start().await;
    // Both instants fall inside the 2026-11-05 Bangkok business day (2026-11-04T23:00:00Z .. 2026-11-05T23:00:00Z).
    set_ops(&shop, "CUT_OFF", Some(cutoff_json("2026-11-05T10:00:00Z"))).await;
    let manager = staff_actor(&shop, "Manager", Role::BranchManager).await;
    let teller = staff_actor(&shop, "Teller", Role::Teller).await;
    let now = "2026-11-05T11:00:00Z";

    let mgr_floor = floor_at(&shop, &manager, now).await;
    assert_eq!(mgr_floor["phase"], json!("CLOSED"), "{mgr_floor}");
    assert_eq!(mgr_floor["openShifts"], json!([]), "{mgr_floor}");

    let teller_floor = floor_at(&shop, &teller, now).await;
    assert_eq!(teller_floor["phase"], json!("CLOSED"), "{teller_floor}");
    assert_eq!(teller_floor["openShifts"], json!([]), "{teller_floor}");

    shop.finish().await;
}

/// Row 4: stored `CUT_OFF`, 0 open shifts, `business_date_of(cutoff.at) < business_date_of(now)` -> `phase` is `NOT_OPENED`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f36_stored_cutoff_with_no_open_shifts_and_yesterdays_cutoff_reads_not_opened() {
    let shop = Shop::start().await;
    // Cutoff on 2026-11-04 Bangkok business day; `now` on 2026-11-05 Bangkok business day.
    set_ops(&shop, "CUT_OFF", Some(cutoff_json("2026-11-04T10:00:00Z"))).await;
    let manager = staff_actor(&shop, "Manager", Role::BranchManager).await;
    let teller = staff_actor(&shop, "Teller", Role::Teller).await;
    let now = "2026-11-05T05:00:00Z";

    let mgr_floor = floor_at(&shop, &manager, now).await;
    assert_eq!(mgr_floor["phase"], json!("NOT_OPENED"), "{mgr_floor}");
    assert_eq!(mgr_floor["openShifts"], json!([]), "{mgr_floor}");

    let teller_floor = floor_at(&shop, &teller, now).await;
    assert_eq!(teller_floor["phase"], json!("NOT_OPENED"), "{teller_floor}");
    assert_eq!(teller_floor["openShifts"], json!([]), "{teller_floor}");

    shop.finish().await;
}

/// Row 5: stored `CUT_OFF`, 0 open shifts, no `cutoff.at` or either business date `None` -> `phase` is `CLOSED` (safe side).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f36_stored_cutoff_with_no_open_shifts_and_missing_cutoff_or_plan_reads_closed() {
    let shop = Shop::start().await;
    let teller = staff_actor(&shop, "Teller", Role::Teller).await;
    let now = "2026-11-05T05:00:00Z";

    // Case A: no cutoff.at on ops_state.
    set_ops(&shop, "CUT_OFF", None).await;
    let floor_no_cutoff = floor_at(&shop, &teller, now).await;
    assert_eq!(
        floor_no_cutoff["phase"],
        json!("CLOSED"),
        "{floor_no_cutoff}"
    );

    // Case B: cutoff.at present (yesterday), but branch has no active shift plan (`business_date_of` is `None`).
    set_ops(&shop, "CUT_OFF", Some(cutoff_json("2026-11-04T10:00:00Z"))).await;
    sqlx::query("UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1")
        .bind(MAIN)
        .execute(&shop.pool)
        .await
        .expect("retire MAIN's templates");
    let floor_no_plan = floor_at(&shop, &teller, now).await;
    assert_eq!(floor_no_plan["phase"], json!("CLOSED"), "{floor_no_plan}");

    shop.finish().await;
}

/// Row 6 (F37a): `OPEN` with two teller shifts -> `phase` is `OPEN` (unchanged); manager (`CashManage`) sees both shifts
/// with teller names in `openShifts`, while a teller without `CashManage` sees `[]`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f37_open_with_two_teller_shifts_lists_both_for_manager_and_none_for_teller() {
    let shop = Shop::start().await;
    set_ops(&shop, "OPEN", None).await;
    let manager = staff_actor(&shop, "Somchai Manager", Role::BranchManager).await;
    let teller_a = staff_actor(&shop, "Ploy Teller", Role::Teller).await;
    let teller_b = staff_actor(&shop, "Nok Teller", Role::Teller).await;

    let shift_a = seed_open_shift(&shop, &teller_a, "2026-11-05T01:00:00Z").await;
    let shift_b = seed_open_shift(&shop, &teller_b, "2026-11-05T01:05:00Z").await;
    let now = "2026-11-05T05:00:00Z";

    let mgr_floor = floor_at(&shop, &manager, now).await;
    assert_eq!(mgr_floor["phase"], json!("OPEN"), "{mgr_floor}");
    assert_eq!(
        mgr_floor["openShifts"],
        json!([
            {
                "shiftId": shift_a,
                "tellerId": teller_a.user_id,
                "tellerName": "Ploy Teller",
                "openedAt": "2026-11-05T01:00:00+00:00",
            },
            {
                "shiftId": shift_b,
                "tellerId": teller_b.user_id,
                "tellerName": "Nok Teller",
                "openedAt": "2026-11-05T01:05:00+00:00",
            },
        ]),
        "{mgr_floor}"
    );

    let teller_floor = floor_at(&shop, &teller_a, now).await;
    assert_eq!(teller_floor["phase"], json!("OPEN"), "{teller_floor}");
    assert_eq!(teller_floor["openShifts"], json!([]), "{teller_floor}");

    shop.finish().await;
}
