//! Up-front tests for ticket CS-5 (docs/design/configurable-shifts.md §2.5, §2.7, T4, §4 rows F1, F2). Written by the
//! ticket author claude/35271802 — do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! The staff floor reads the shift plan: `hours` is the plan's business day holding `now` (T4), not
//! `branches.opening_hours`; `myShift` also carries the caller's `shiftName`, `businessDate` and `onBreak`
//! (`ShiftPlan::on_break` for the template of the caller's open shift, §2.5, display only).
//!
//! Seam: `StaffFloorReader::floor_at(actor, branch, now)`, the floor as of `now` (`floor` is it at `Utc::now()`).
//! The new `myShift` fields are read from the serialised view (camelCase, as the web receives it), so the wire names
//! are pinned too.
//!
//! Fixture "overlap24", stored by SQL: A 06:00–15:00, B 14:00–23:00 (break 18:00–19:00), C 22:00–07:00, cutover 06:00,
//! Asia/Bangkok. Instants are written in UTC; Bangkok is UTC+7, so 4 Nov 18:30 Bangkok is `2026-11-04T11:30:00Z`.

#[allow(unused_imports)]
mod save_common;

use chrono::{DateTime, Utc};
use save_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor, Role};
use xchange::application::staff_floor::StaffFloorReader;
use xchange::presentation::test_callers;

/// `(name, start, end, break start, break end)` of one template.
type TemplateRow<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, Option<&'a str>);

/// Replaces MAIN's live plan with overlap24 (B has a break 18:00–19:00).
async fn overlap24(shop: &Shop) {
    sqlx::query("UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1 AND retired_at IS NULL")
        .bind(MAIN)
        .execute(&shop.pool)
        .await
        .expect("retire MAIN's plan");
    sqlx::query("UPDATE branches SET timezone = 'Asia/Bangkok' WHERE id = $1")
        .bind(MAIN)
        .execute(&shop.pool)
        .await
        .expect("MAIN in Bangkok");
    let rows: [TemplateRow; 3] = [
        ("A", "06:00", "15:00", None, None),
        ("B", "14:00", "23:00", Some("18:00"), Some("19:00")),
        ("C", "22:00", "07:00", None, None),
    ];
    for (i, (name, start, end, break_start, break_end)) in rows.iter().enumerate() {
        sqlx::query(
            "INSERT INTO shift_templates (organization_id, branch_id, name, starts_at, ends_at, \
                 break_starts_at, break_ends_at, position) \
             SELECT organization_id, id, $2, $3::time, $4::time, $5::time, $6::time, $7 FROM branches WHERE id = $1",
        )
        .bind(MAIN)
        .bind(name)
        .bind(start)
        .bind(end)
        .bind(break_start)
        .bind(break_end)
        .bind(i as i16)
        .execute(&shop.pool)
        .await
        .expect("insert a template");
    }
}

/// A teller of MAIN's organisation (`users.id`) and the actor the floor sees for them.
async fn teller(shop: &Shop, name: &str) -> Actor {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT organization_id, $2, 'ACTIVE', 'test' FROM branches WHERE id = $1 RETURNING id::text",
    )
    .bind(MAIN)
    .bind(name)
    .fetch_one(&shop.pool)
    .await
    .expect("insert a user");
    test_callers::staff(&id, &[Role::Teller], branch(MAIN))
}

/// An OPEN shift of `who` at MAIN, opened under the live template `template` on business date 2026-11-04.
async fn open_shift_as(shop: &Shop, who: &Actor, template: &str, opened_at: &str) -> String {
    let id = format!("SH-{}", unique("cs5"));
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id, \
             template_id, template_name, business_date) \
         SELECT $1, $2, 'OPEN', $3, 'test', '', $4, $5::uuid, t.id, t.name, DATE '2026-11-04' \
           FROM shift_templates t WHERE t.branch_id = $2 AND t.name = $6 AND t.retired_at IS NULL",
    )
    .bind(&id)
    .bind(MAIN)
    .bind(instant(opened_at))
    .bind(unique("cs5-open").replace('_', "-"))
    .bind(&who.user_id)
    .bind(template)
    .execute(&shop.pool)
    .await
    .expect("seed an OPEN shift under a template");
    id
}

/// The floor `who` sees at `now`, serialised as the view answers it.
async fn floor_at(shop: &Shop, who: &Actor, now: &str) -> Value {
    let floor = StaffFloorReader::new(shop.pool.clone())
        .floor_at(who, &branch(MAIN), instant(now))
        .await
        .expect("the floor");
    serde_json::to_value(&floor).expect("the floor serialises")
}

fn time(v: &Value, what: &str) -> DateTime<Utc> {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a time string, got {v}"));
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("{what} {s:?}: {e}"))
        .with_timezone(&Utc)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f1_the_hours_are_the_business_day_holding_now() {
    let shop = Shop::start().await;
    overlap24(&shop).await;
    let me = teller(&shop, "F1").await;
    // 5 Nov 02:00 Bangkok belongs to the 4 Nov business day: 4 Nov 06:00 → 5 Nov 06:00.
    let floor = floor_at(&shop, &me, "2026-11-04T19:00:00Z").await;
    assert_eq!(
        (
            time(&floor["hours"]["open"], "hours.open"),
            time(&floor["hours"]["close"], "hours.close")
        ),
        (
            instant("2026-11-03T23:00:00Z"),
            instant("2026-11-04T23:00:00Z")
        ),
        "F1: the 4 Nov business day: {floor}"
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f1b_opening_hours_are_not_the_floors_hours() {
    let shop = Shop::start().await;
    overlap24(&shop).await;
    let day = json!([{ "open": "12:00", "close": "18:00" }]);
    sqlx::query("UPDATE branches SET opening_hours = $2 WHERE id = $1")
        .bind(MAIN)
        .bind(json!({"mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": day}))
        .execute(&shop.pool)
        .await
        .expect("set MAIN's opening hours");
    let me = teller(&shop, "F1b").await;
    // 4 Nov 13:00 Bangkok: inside 12–18, but the floor's hours are still the plan's business day.
    let floor = floor_at(&shop, &me, "2026-11-04T06:00:00Z").await;
    assert_eq!(
        (
            time(&floor["hours"]["open"], "hours.open"),
            time(&floor["hours"]["close"], "hours.close")
        ),
        (
            instant("2026-11-03T23:00:00Z"),
            instant("2026-11-04T23:00:00Z")
        ),
        "F1b: opening_hours 12:00–18:00 do not gate the floor: {floor}"
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f2_my_shift_names_its_shift_and_date_and_is_on_break_only_inside_its_break() {
    let shop = Shop::start().await;
    overlap24(&shop).await;
    let b_teller = teller(&shop, "in B").await;
    let a_teller = teller(&shop, "in A").await;
    let b_shift = open_shift_as(&shop, &b_teller, "B", "2026-11-04T07:05:00Z").await;
    let a_shift = open_shift_as(&shop, &a_teller, "A", "2026-11-03T23:05:00Z").await;

    // 18:30 Bangkok: inside B's break.
    let floor = floor_at(&shop, &b_teller, "2026-11-04T11:30:00Z").await;
    let mine = &floor["myShift"];
    assert_eq!(mine["id"], json!(b_shift), "{floor}");
    assert_eq!(mine["shiftName"], json!("B"), "F2: the shift's name: {floor}");
    assert_eq!(
        mine["businessDate"],
        json!("2026-11-04"),
        "F2: the shift's business date: {floor}"
    );
    assert_eq!(mine["onBreak"], json!(true), "F2: B at 18:30 is on break: {floor}");

    // 14:30 Bangkok: B has started, its break has not.
    let floor = floor_at(&shop, &b_teller, "2026-11-04T07:30:00Z").await;
    assert_eq!(
        floor["myShift"]["onBreak"],
        json!(false),
        "F2: B at 14:30 is not on break: {floor}"
    );

    // 18:30 Bangkok for a teller still holding an A shift: A has no break.
    let floor = floor_at(&shop, &a_teller, "2026-11-04T11:30:00Z").await;
    assert_eq!(floor["myShift"]["id"], json!(a_shift), "{floor}");
    assert_eq!(floor["myShift"]["shiftName"], json!("A"), "{floor}");
    assert_eq!(
        floor["myShift"]["onBreak"],
        json!(false),
        "F2: a teller in A is not on break at 18:30: {floor}"
    );
    shop.finish().await;
}
