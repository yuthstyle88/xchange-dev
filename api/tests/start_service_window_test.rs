//! #4130 (owner 2026-10-21, "ทำตามที่แนะนำทั้ง 5 ข้อ", recommendation 2): service cannot start outside the
//! branch's hours. `BranchDayService::start_service` must refuse a READY day when
//! `ServiceWindow::allows_start(now)` is false, with the same sentence #755 pins for a refused start
//! (`CustodyError::ServiceNotReady { window, utc_offset_minutes }`), and leave the day READY.
//!
//! Written before the implementation by the BE-R6 sub-controller (claude/studio-claude-8). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The command goes the way the staff route sends it: `BranchDayService` on PostgreSQL (`PgCustodyPorts` reads the
//! branch's `timezone` and `opening_hours`), by a BRANCH_MANAGER of `MAIN` whose day is READY. Rows:
//! 1. inside the window: READY → OPEN (unchanged);
//! 2. before opening, and at/after the last start (close − 10 min): refused with the hours, still READY;
//! 3. hours not set, and a day the branch is closed: refused with #755's sentence, still READY;
//! 4. a day that is not READY keeps today's refusal: pinned by `service_not_ready_message_test.rs` (CLOSED day) and
//!    `crates/xc-shift/tests/branch_day_test.rs` (every other phase); not repeated here.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[allow(unused_imports)]
mod save_common;

use chrono::{TimeZone, Utc};
use save_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor, Role as BpRole, ScopedRole};
use xc_shift::application::branch_day::service::{BranchDayService, StartService};
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

const NOT_SET: &str = "สาขานี้ยังไม่ได้ตั้งเวลาทำการ ให้ผู้ดูแลตั้งเวลาก่อนเปิดบริการ";
const CLOSED_TODAY: &str = "วันนี้สาขาปิด";

fn ready_between(open: &str, last_start: &str) -> String {
    format!("เปิดบริการได้หลังเตรียมกะเสร็จ ระหว่าง {open}–{last_start}")
}

async fn set_hours(shop: &Shop, timezone: &str, hours: Option<Value>) {
    sqlx::query("UPDATE branches SET timezone = $2, opening_hours = $3 WHERE id = $1")
        .bind(MAIN)
        .bind(timezone)
        .bind(hours)
        .execute(&shop.pool)
        .await
        .expect("set the branch's hours");
}

/// `MAIN`'s stored day is READY.
async fn day_is_ready(shop: &Shop) {
    sqlx::query(
        "INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'READY') \
         ON CONFLICT (branch_id) DO UPDATE SET phase = 'READY', service_opened_at = NULL",
    )
    .bind(MAIN)
    .execute(&shop.pool)
    .await
    .expect("the day is READY");
}

async fn stored_phase(shop: &Shop) -> String {
    sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
        .bind(MAIN)
        .fetch_one(&shop.pool)
        .await
        .expect("the day's row")
}

/// The same hours Monday to Saturday; Sunday closed.
fn week(open: &str, close: &str) -> Value {
    let day = json!([{ "open": open, "close": close }]);
    json!({
        "mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": [],
    })
}

/// A BRANCH_MANAGER of `MAIN`.
fn manager() -> Actor {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        "00000000-0000-4000-8000-0000000b0001",
        "00000000-0000-0000-0000-000000000001",
        vec![ScopedRole::new(
            BpRole::BranchManager,
            Some(branch(MAIN)),
            from,
            None,
        )],
        None,
    )
}

/// Starts the service of `MAIN` at `now` (an ISO instant) and commits what it wrote.
async fn start_at(shop: &Shop, now: &str) -> pos_error::AppResult<Phase> {
    let days = BranchDayService::new(
        std::sync::Arc::new(PgCustodyRepository::new(shop.pool.clone())),
        shop.shifts.clone(),
        std::sync::Arc::new(PgCustodyPorts),
    );
    let mut tx = shop.pool.begin().await.expect("begin");
    let got = days
        .start_service(
            &mut tx,
            &manager(),
            &branch(MAIN),
            StartService {
                equipment_ready: true,
                rates_verified: true,
                rate_updated: "2026-10-06T03:00:00.000Z".to_owned(),
            },
            instant(now),
        )
        .await;
    tx.commit().await.expect("commit");
    got
}

/// Row 1: inside the window the start is unchanged — READY → OPEN.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_day_inside_the_window_opens() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    // Wednesday 2026-10-07: 10:00 (opening, inclusive), 14:00, and 19:49 in Tokyo (01:00, 05:00, 10:49 UTC)
    for now in [
        "2026-10-07T01:00:00.000Z",
        "2026-10-07T05:00:00.000Z",
        "2026-10-07T10:49:00.000Z",
    ] {
        day_is_ready(&shop).await;
        let got = start_at(&shop, now).await;
        assert!(matches!(got, Ok(Phase::Open)), "{now}: got {got:?}");
        assert_eq!(stored_phase(&shop).await, "OPEN", "{now}");
    }
    shop.finish().await;
}

/// Row 2: before opening, and from the last start (close − 10 min) on, the start is refused with the branch's
/// hours and the day stays READY.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_day_outside_the_window_is_refused_with_the_hours_and_stays_ready() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    day_is_ready(&shop).await;
    // Wednesday 2026-10-07 in Tokyo: 09:00, 09:59 (before opening), 19:50 (the last start, exclusive),
    // 19:55 and 21:00 (after it)
    for now in [
        "2026-10-07T00:00:00.000Z",
        "2026-10-07T00:59:00.000Z",
        "2026-10-07T10:50:00.000Z",
        "2026-10-07T10:55:00.000Z",
        "2026-10-07T12:00:00.000Z",
    ] {
        assert_eq!(
            refusal(start_at(&shop, now).await),
            ready_between("10:00", "19:50"),
            "{now}"
        );
        assert_eq!(stored_phase(&shop).await, "READY", "{now}: nothing changed");
    }
    shop.finish().await;
}

/// Row 3a: a branch without hours cannot start, and is told to have them set.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_day_of_a_branch_without_hours_is_refused() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", None).await;
    day_is_ready(&shop).await;
    assert_eq!(
        refusal(start_at(&shop, "2026-10-07T05:00:00.000Z").await),
        NOT_SET
    );
    assert_eq!(stored_phase(&shop).await, "READY");
    shop.finish().await;
}

/// Row 3b: on a day the branch is closed the start is refused, whatever the time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_day_on_a_closed_day_is_refused() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    day_is_ready(&shop).await;
    // Sunday 2026-10-11 in Tokyo (`"sun": []`): 12:00, inside the other days' hours
    assert_eq!(
        refusal(start_at(&shop, "2026-10-11T03:00:00.000Z").await),
        CLOSED_TODAY
    );
    assert_eq!(stored_phase(&shop).await, "READY");
    shop.finish().await;
}
