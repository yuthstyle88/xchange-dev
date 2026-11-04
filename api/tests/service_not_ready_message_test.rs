//! #755, restated for CS-4m (#6241) by the configurable-shifts sub-controller claude/35271802: a refused service start
//! tells staff why, in words that are true on a shift plan.
//!
//! Ruling (lane controller claude/022a7453 on #6241, from owner Q7 = A "no daily pause", Q5 = A "selling follows the
//! shifts"): after CS-4 the custody window is the plan's business day joined with the next (design
//! `docs/design/configurable-shifts.md` T4, T10), and every plan covers 24 hours (T5). So there is no last start and no
//! closed day to name:
//! - a branch with a plan: `เปิดบริการได้หลังเตรียมกะเสร็จ`, at any time, in any timezone;
//! - a branch with no live template: `สาขานี้ยังไม่ได้ตั้งกะ ให้ผู้ดูแลตั้งกะก่อนเปิดบริการ`.
//!
//! The seven tests keep #755's cases (own timezone, across the hour, past midnight, v2's Bangkok week, no hours,
//! before/after the old hours, the old closed Sunday) and re-expect them. Written before CS-4m's implementation. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! The start goes the way the staff route sends it: CA-8b's `BranchDayService::start_service` on `PgCustodyPorts`, by a
//! BRANCH_MANAGER of `MAIN` while the day is CLOSED, so it is refused `ServiceNotReady` whatever the time; what is
//! pinned is the sentence of that refusal.
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[allow(unused_imports)]
mod save_common;
mod shift_plan_common;

use chrono::{TimeZone, Utc};
use save_common::*;
use serde_json::json;
use shift_plan_common::{clear_plan, set_plan, set_timezone};
use xc_auth::domain::access::{Actor, Role as BpRole, ScopedRole};
use xc_shift::application::branch_day::service::{BranchDayService, StartService};
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

const NOT_SET: &str = "สาขานี้ยังไม่ได้ตั้งกะ ให้ผู้ดูแลตั้งกะก่อนเปิดบริการ";
const READY_AFTER_SETUP: &str = "เปิดบริการได้หลังเตรียมกะเสร็จ";

/// A BRANCH_MANAGER of `MAIN` (no row is needed: `BranchDay::start_service` asks the actor's own roles).
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

/// The sentence a BRANCH_MANAGER is refused with when starting the service of `MAIN`, whose day is CLOSED, at `now`
/// (an ISO instant).
async fn refused_start(shop: &Shop, now: &str) -> String {
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
    tx.rollback().await.expect("rollback");
    refusal(got)
}

/// `opening_hours` set to `open`–`close` every day: customer-facing only (Q5 = A), it must not reach the sentence.
async fn customer_hours(shop: &Shop, open: &str, close: &str) {
    let day = json!([{ "open": open, "close": close }]);
    sqlx::query("UPDATE branches SET opening_hours = $2 WHERE id = $1")
        .bind(MAIN)
        .bind(json!({
            "mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": [],
        }))
        .execute(&shop.pool)
        .await
        .expect("set the customer-facing hours");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sentence_names_no_window_whatever_the_branchs_timezone() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    customer_hours(&shop, "10:00", "20:00").await;
    // Wednesday 2026-10-07, 14:00 and 19:55 in Tokyo (05:00 and 10:55 UTC)
    for now in ["2026-10-07T05:00:00.000Z", "2026-10-07T10:55:00.000Z"] {
        assert_eq!(refused_start(&shop, now).await, READY_AFTER_SETUP, "{now}");
    }
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn there_is_no_last_start_before_the_cutover_on_a_24_hour_plan() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    // Thursday 2026-10-08, 05:50 and 05:55 in Tokyo: the old "last start" 10 minutes before the 06:00 cutover
    for now in ["2026-10-07T20:50:00.000Z", "2026-10-07T20:55:00.000Z"] {
        assert_eq!(refused_start(&shop, now).await, READY_AFTER_SETUP, "{now}");
    }
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shift_past_midnight_names_no_window_either() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    set_plan(
        &shop.pool,
        MAIN,
        &[("Day", "06:00", "18:00"), ("Night", "18:00", "06:00")],
    )
    .await;
    // Wednesday 19:00 and Thursday 01:55 in Tokyo, both inside Wednesday's Night 18:00–06:00
    for now in ["2026-10-07T10:00:00.000Z", "2026-10-07T16:55:00.000Z"] {
        assert_eq!(refused_start(&shop, now).await, READY_AFTER_SETUP, "{now}");
    }
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_v2_week_in_bangkok_no_longer_reads_eleven_to_ten_to_eleven() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Bangkok").await;
    customer_hours(&shop, "11:00", "23:00").await;
    // Wednesday 2026-10-07, 13:25 in Bangkok
    assert_eq!(
        refused_start(&shop, "2026-10-07T06:25:00.000Z").await,
        READY_AFTER_SETUP
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_without_shifts_is_told_to_have_them_set() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    clear_plan(&shop.pool, MAIN).await;
    assert_eq!(
        refused_start(&shop, "2026-10-07T05:00:00.000Z").await,
        NOT_SET
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_and_after_the_customer_hours_the_sentence_is_the_same() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    customer_hours(&shop, "10:00", "20:00").await;
    // Wednesday 2026-10-07, 09:00 and 21:00 in Tokyo: outside the customer hours, inside the business day
    for now in ["2026-10-07T00:00:00.000Z", "2026-10-07T12:00:00.000Z"] {
        assert_eq!(refused_start(&shop, now).await, READY_AFTER_SETUP, "{now}");
    }
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_day_closed_to_customers_is_not_a_closed_day() {
    let shop = Shop::start().await;
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    customer_hours(&shop, "10:00", "20:00").await;
    // Sunday 2026-10-11 in Tokyo (`"sun": []` for customers): 09:00, 12:00 and 21:00 are all in a business day
    for now in [
        "2026-10-11T00:00:00.000Z",
        "2026-10-11T03:00:00.000Z",
        "2026-10-11T12:00:00.000Z",
    ] {
        assert_eq!(refused_start(&shop, now).await, READY_AFTER_SETUP, "{now}");
    }
    shop.finish().await;
}
