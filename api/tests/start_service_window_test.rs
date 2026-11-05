//! #4130 (owner 2026-10-21, "ทำตามที่แนะนำทั้ง 5 ข้อ", recommendation 2), restated for CS-4s (#6236) by claude/35271802.
//!
//! Under Q5 = A (selling follows the shifts; opening_hours is customer-facing only), Q7 = A (no daily pause), and T5
//! (every plan covers 24 h), service start follows the shift plan's business day, not opening_hours.
//!
//! The command goes the way the staff route sends it: `BranchDayService` on PostgreSQL (`PgCustodyPorts` reads the
//! branch's shift templates and `timezone`), by a BRANCH_MANAGER of `MAIN` whose day is READY. Rows:
//! 1. inside the former window: READY → OPEN (unchanged);
//! 2. former outside-the-window times: now in the 24 h plan, so READY → OPEN;
//! 3. branch with no live template: refused with `NOT_SET`, stays READY;
//! 4. former closed day (Sunday): covered by the plan, so READY → OPEN.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[allow(unused_imports)]
mod save_common;
mod shift_plan_common;

use chrono::{TimeZone, Utc};
use save_common::*;
use shift_plan_common::{clear_plan, set_timezone};
use xc_auth::domain::access::{Actor, Role as BpRole, ScopedRole};
use xc_shift::application::branch_day::service::{BranchDayService, StartService};
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

const NOT_SET: &str = "สาขานี้ยังไม่ได้ตั้งกะ ให้ผู้ดูแลตั้งกะก่อนเปิดบริการ";

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
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
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

/// Row 2: former outside-the-window times now open under the 24 h plan.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn former_outside_the_window_times_now_open() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    // Wednesday 2026-10-07 in Tokyo: 09:00, 09:59 (before former opening), 19:50 (former last start),
    // 19:55 and 21:00 (after it), and Thursday 05:55 (just before the 06:00 cutover)
    for now in [
        "2026-10-07T00:00:00.000Z",
        "2026-10-07T00:59:00.000Z",
        "2026-10-07T10:50:00.000Z",
        "2026-10-07T10:55:00.000Z",
        "2026-10-07T12:00:00.000Z",
        "2026-10-07T20:55:00.000Z",
    ] {
        day_is_ready(&shop).await;
        let got = start_at(&shop, now).await;
        assert!(matches!(got, Ok(Phase::Open)), "{now}: got {got:?}");
        assert_eq!(stored_phase(&shop).await, "OPEN", "{now}");
    }
    shop.finish().await;
}

/// Row 3: a branch without live templates cannot start, and is told to have them set.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_day_of_a_branch_without_hours_is_refused() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    clear_plan(&shop.pool, MAIN).await;
    day_is_ready(&shop).await;
    assert_eq!(
        refusal(start_at(&shop, "2026-10-07T05:00:00.000Z").await),
        NOT_SET
    );
    assert_eq!(stored_phase(&shop).await, "READY");
    shop.finish().await;
}

/// Row 4: a former closed day (Sunday) now opens under the 24 h plan.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn former_closed_day_now_opens() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    day_is_ready(&shop).await;
    // Sunday 2026-10-11 in Tokyo: 12:00 (03:00 UTC)
    let got = start_at(&shop, "2026-10-11T03:00:00.000Z").await;
    assert!(matches!(got, Ok(Phase::Open)), "got {got:?}");
    assert_eq!(stored_phase(&shop).await, "OPEN");
    shop.finish().await;
}
