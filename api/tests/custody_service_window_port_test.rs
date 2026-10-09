//! OR-6 (`docs/design/blueprint/org-roles-branches.md` §6): `PgCustodyPorts::service_window`
//! reads the branch's own `timezone` and `opening_hours` (OR-1, `BranchHours::window`) and
//! answers the window that the custody machine judges a command by.
//!
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! What is pinned:
//! - a time inside an open window of the branch answers exactly that window (`open`, `close`
//!   as `BranchHours::window` gives them), in the branch's timezone, including the window of
//!   the previous local day that runs past midnight, east and west of UTC;
//! - `opening_hours` NULL answers `NotSet`;
//! - a time outside every window answers no window that contains it (and not `NotSet`).
//!
//! This file does not compile before OR-6: `ServiceWindow` and
//! `CustodyPorts::service_window` are what the ticket adds.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use save_common::*;
use serde_json::{Value, json};
use xc_shift::domain::custody::repositories::custody_ports::CustodyPorts;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

fn ports() -> Arc<dyn CustodyPorts> {
    Arc::new(PgCustodyPorts)
}

fn ct(iso: &str) -> CustodyTime {
    let t: DateTime<Utc> = instant(iso);
    CustodyTime::from_epoch_millis(t.timestamp_millis())
}

async fn window(shop: &Shop, branch_id: &str, now: &str) -> ServiceWindow {
    let mut tx = shop.pool.begin().await.expect("begin");
    let got = ports()
        .service_window(&mut tx, &branch(branch_id), ct(now))
        .await
        .expect("service_window");
    tx.rollback().await.expect("rollback");
    got
}

async fn set_hours(shop: &Shop, branch_id: &str, timezone: &str, hours: Option<Value>) {
    sqlx::query("UPDATE branches SET timezone = $2, opening_hours = $3 WHERE id = $1")
        .bind(branch_id)
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

fn contains(w: ServiceWindow, now: &str) -> bool {
    match w {
        ServiceWindow::Open { open, close } => open <= ct(now) && ct(now) < close,
        ServiceWindow::NotSet | ServiceWindow::Closed => false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_without_hours_has_no_window() {
    let shop = Shop::start().await;
    // the migration leaves `opening_hours` NULL (B4: "not set")
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T05:00:00.000Z").await,
        ServiceWindow::NotSet
    );
    // another branch's hours are not this branch's
    shop.seed_branch("XC-B02", "Tokyo").await;
    set_hours(&shop, "XC-B02", "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T05:00:00.000Z").await,
        ServiceWindow::NotSet
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_window_is_the_branchs_hours_in_its_own_timezone() {
    let shop = Shop::start().await;
    set_hours(&shop, MAIN, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    // Wednesday 2026-10-07, 14:00 in Tokyo
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T05:00:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-07T01:00:00.000Z"),
            close: ct("2026-10-07T11:00:00.000Z"),
        }
    );
    // 10:00 and 19:59:59 in Tokyo are inside; 09:59 and 20:00 are not
    for now in ["2026-10-07T01:00:00.000Z", "2026-10-07T10:59:59.000Z"] {
        assert!(contains(window(&shop, MAIN, now).await, now), "{now}");
    }
    for now in ["2026-10-07T00:59:00.000Z", "2026-10-07T11:00:00.000Z"] {
        let w = window(&shop, MAIN, now).await;
        assert!(!contains(w, now), "{now}: {w:?}");
        assert_ne!(w, ServiceWindow::NotSet, "{now}: the hours are set");
    }

    // the same wall-clock hours in Bangkok are other instants
    set_hours(&shop, MAIN, "Asia/Bangkok", Some(week("10:00", "20:00"))).await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T12:30:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-07T03:00:00.000Z"),
            close: ct("2026-10-07T13:00:00.000Z"),
        }
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_day_has_no_window_that_contains_its_time() {
    let shop = Shop::start().await;
    set_hours(&shop, MAIN, "Asia/Tokyo", Some(week("10:00", "20:00"))).await;
    // Sunday 2026-10-11, 12:00 in Tokyo: `"sun": []`
    let now = "2026-10-11T03:00:00.000Z";
    let w = window(&shop, MAIN, now).await;
    assert!(!contains(w, now), "{w:?}");
    assert_ne!(w, ServiceWindow::NotSet, "the hours are set");
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_window_past_midnight_belongs_to_the_day_it_opened() {
    let shop = Shop::start().await;
    set_hours(&shop, MAIN, "Asia/Tokyo", Some(week("18:00", "02:00"))).await;
    // Thursday 2026-10-08, 01:30 in Tokyo: inside Wednesday's 18:00–02:00
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T16:30:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-07T09:00:00.000Z"),
            close: ct("2026-10-07T17:00:00.000Z"),
        }
    );
    // west of UTC: Thursday 2026-10-08, 01:00 in Los Angeles (UTC−7) is 08:00 UTC on the 8th,
    // inside Wednesday's 18:00–02:00 local, which is not the UTC date's own window
    set_hours(
        &shop,
        MAIN,
        "America/Los_Angeles",
        Some(week("18:00", "02:00")),
    )
    .await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-08T08:00:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-08T01:00:00.000Z"),
            close: ct("2026-10-08T09:00:00.000Z"),
        }
    );
    set_hours(&shop, MAIN, "Asia/Tokyo", Some(week("18:00", "02:00"))).await;
    // Monday 2026-10-12, 01:30 in Tokyo: Sunday is closed, so nothing contains it
    let now = "2026-10-11T16:30:00.000Z";
    let w = window(&shop, MAIN, now).await;
    assert!(!contains(w, now), "{w:?}");
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_branchs_day_can_be_ahead_of_the_utc_day() {
    let shop = Shop::start().await;
    set_hours(&shop, MAIN, "Asia/Tokyo", Some(week("08:00", "12:00"))).await;
    // Thursday 2026-10-08, 08:30 in Tokyo is still Wednesday 23:30 in UTC
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T23:30:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-07T23:00:00.000Z"),
            close: ct("2026-10-08T03:00:00.000Z"),
        }
    );
    shop.finish().await;
}
