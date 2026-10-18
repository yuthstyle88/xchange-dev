//! #755: a refused service start tells staff the window of *their* branch, in the branch's own
//! local time, instead of v2's fixed `11:00–22:50`.
//!
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! The command goes the way the staff route sends it: `CustodyService` built on PostgreSQL
//! (`pg_custody_service`, so `PgCustodyPorts` reads the branch's `timezone` and
//! `opening_hours`). The desk is enabled with its back-office employee clocked in and no shift
//! open, so `opsStartService` is refused as not ready whatever the time; what is pinned is the
//! sentence of that refusal:
//! - an open window: `เปิดบริการได้หลังเตรียมกะเสร็จ ระหว่าง {open}–{close − 10 min}`, both as
//!   `HH:MM` in the branch's timezone;
//! - hours not set: `สาขานี้ยังไม่ได้ตั้งเวลาทำการ ให้ผู้ดูแลตั้งเวลาก่อนเปิดบริการ`;
//! - a day the branch is closed: `วันนี้สาขาปิด`.
//!
//! Lane controller's ruling on #755: when the branch's local day has a window, the sentence
//! names that window also before it opens and after it closes; `วันนี้สาขาปิด` is only for a
//! day with no window at all. (The port answers that day's window for display; the machine
//! still refuses outside it.)
//!
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c; cash-shift-transfer.md §6, ruling Q7): v2's
//! `opsStartService` goes with the custody command engine, so the start is CA-8b's: `BranchDayService::start_service`
//! on `PgCustodyPorts` (which reads the same `timezone` and `opening_hours`), by a BRANCH_MANAGER of `MAIN` while the
//! day is CLOSED, so it is refused `ServiceNotReady` whatever the time. The pinned sentences are unchanged.
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[allow(unused_imports)]
mod save_common;

use chrono::{TimeZone, Utc};
use save_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor, Role as BpRole, ScopedRole};
use xc_shift::application::branch_day::service::{BranchDayService, StartService};
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

/// The same hours Monday to Saturday; Sunday closed.
fn week(open: &str, close: &str) -> Value {
    let day = json!([{ "open": open, "close": close }]);
    json!({
        "mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": [],
    })
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sentence_names_the_branchs_own_window_in_its_own_timezone() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    // Wednesday 2026-10-07, 14:00 and 19:55 in Tokyo (05:00 and 10:55 UTC)
    for now in ["2026-10-07T05:00:00.000Z", "2026-10-07T10:55:00.000Z"] {
        assert_eq!(
            refused_start(&shop, now).await,
            ready_between("10:00", "19:50"),
            "{now}"
        );
    }
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_last_start_is_ten_minutes_before_closing_across_the_hour() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("10:00", "20:05"))).await;
    // Wednesday 2026-10-07, 20:00 in Tokyo
    assert_eq!(
        refused_start(&shop, "2026-10-07T11:00:00.000Z").await,
        ready_between("10:00", "19:55")
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_window_past_midnight_is_shown_as_the_local_times_it_has() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("18:00", "02:00"))).await;
    // Wednesday 19:00 and Thursday 01:55 in Tokyo, both inside Wednesday's 18:00–02:00
    for now in ["2026-10-07T10:00:00.000Z", "2026-10-07T16:55:00.000Z"] {
        assert_eq!(
            refused_start(&shop, now).await,
            ready_between("18:00", "01:50"),
            "{now}"
        );
    }
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_v2_week_in_bangkok_still_reads_eleven_to_ten_to_eleven() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Bangkok", Some(week("11:00", "23:00"))).await;
    // Wednesday 2026-10-07, 13:25 in Bangkok
    assert_eq!(
        refused_start(&shop, "2026-10-07T06:25:00.000Z").await,
        ready_between("11:00", "22:50")
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_without_hours_is_told_to_have_them_set() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", None).await;
    assert_eq!(
        refused_start(&shop, "2026-10-07T05:00:00.000Z").await,
        NOT_SET
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_opening_and_after_closing_the_sentence_names_todays_window() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    // Wednesday 2026-10-07, 09:00 and 21:00 in Tokyo: the day has a window, it is not "closed"
    for now in ["2026-10-07T00:00:00.000Z", "2026-10-07T12:00:00.000Z"] {
        assert_eq!(
            refused_start(&shop, now).await,
            ready_between("10:00", "19:50"),
            "{now}"
        );
    }
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_closed_today_is_told_so() {
    let shop = Shop::start().await;
    set_hours(&shop, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    // Sunday 2026-10-11 in Tokyo, `"sun": []` while the other days are open: 09:00, 12:00
    // and 21:00 all read as a closed day
    for now in [
        "2026-10-11T00:00:00.000Z",
        "2026-10-11T03:00:00.000Z",
        "2026-10-11T12:00:00.000Z",
    ] {
        assert_eq!(refused_start(&shop, now).await, CLOSED_TODAY, "{now}");
    }
    shop.finish().await;
}
