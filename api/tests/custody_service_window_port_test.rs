//! CS-4 (`docs/design/configurable-shifts.md` §2.7, T4, T10): `PgCustodyPorts::service_window`
//! reads the branch's own `timezone` and shift plan and answers the window that the custody
//! machine judges a command by: the business day's start joined with the next business day's end.
//!
//! Restated for CS-4s by claude/35271802 (replaces OR-6's opening_hours with the shift plan).
//!
//! What is pinned:
//! - the window is `Open { business day's start, next business day's end }` = local `[d 06:00, d+2 06:00)`
//!   in the branch's timezone, including times past midnight that belong to the previous business day;
//! - a branch without a live template answers `NotSet`;
//! - every day is covered (no closed day);
//! - another branch's plan is not this branch's.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[allow(unused_imports)]
mod save_common;
mod shift_plan_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use save_common::*;
use shift_plan_common::{clear_plan, set_timezone};
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

fn contains(w: ServiceWindow, now: &str) -> bool {
    match w {
        ServiceWindow::Open { open, close } => open <= ct(now) && ct(now) < close,
        ServiceWindow::NotSet | ServiceWindow::Closed => false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_without_a_live_template_has_no_window() {
    let shop = Shop::start().await;
    // clear_plan(MAIN); XC-B02 (Tokyo) keeps its defaults
    clear_plan(&shop.pool, MAIN).await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T05:00:00.000Z").await,
        ServiceWindow::NotSet
    );
    // another branch's plan is not MAIN's
    shop.seed_branch("XC-B02", "Tokyo").await;
    set_timezone(&shop.pool, "XC-B02", "Asia/Tokyo").await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T05:00:00.000Z").await,
        ServiceWindow::NotSet
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_window_is_the_branchs_plan_in_its_own_timezone() {
    let shop = Shop::start().await;
    // P2: own timezone (Asia/Tokyo), Wednesday 2026-10-07, 14:00 JST
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T05:00:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-06T21:00:00.000Z"),
            close: ct("2026-10-08T21:00:00.000Z"),
        }
    );

    // P3: same, Bangkok (Asia/Bangkok), Wednesday 2026-10-07, 19:30 ICT
    set_timezone(&shop.pool, MAIN, "Asia/Bangkok").await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T12:30:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-06T23:00:00.000Z"),
            close: ct("2026-10-08T23:00:00.000Z"),
        }
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_day_is_covered_there_is_no_closed_day_any_more() {
    let shop = Shop::start().await;
    // P4: a_closed_day_… → no closed day any more (tz Asia/Tokyo, Sunday 2026-10-11, 12:00 JST)
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    let now = "2026-10-11T03:00:00.000Z";
    let expected = ServiceWindow::Open {
        open: ct("2026-10-10T21:00:00.000Z"),
        close: ct("2026-10-12T21:00:00.000Z"),
    };
    let w = window(&shop, MAIN, now).await;
    assert_eq!(w, expected);
    assert!(contains(w, now), "{now}: {w:?}");
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn past_midnight_belongs_to_the_business_day_it_started() {
    let shop = Shop::start().await;
    // P5: Thursday 2026-10-08, 01:30 JST is inside Wednesday's business day
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T16:30:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-06T21:00:00.000Z"),
            close: ct("2026-10-08T21:00:00.000Z"),
        }
    );

    // P6: west of UTC: Thursday 2026-10-08, 01:00 PDT (UTC−7) is 08:00 UTC on the 8th,
    // inside Wednesday's business day
    set_timezone(&shop.pool, MAIN, "America/Los_Angeles").await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-08T08:00:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-07T13:00:00.000Z"),
            close: ct("2026-10-09T13:00:00.000Z"),
        }
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_branchs_day_can_be_ahead_of_the_utc_day() {
    let shop = Shop::start().await;
    // P7: Thursday 2026-10-08, 08:30 in Tokyo is still Wednesday 23:30 in UTC
    set_timezone(&shop.pool, MAIN, "Asia/Tokyo").await;
    assert_eq!(
        window(&shop, MAIN, "2026-10-07T23:30:00.000Z").await,
        ServiceWindow::Open {
            open: ct("2026-10-07T21:00:00.000Z"),
            close: ct("2026-10-09T21:00:00.000Z"),
        }
    );
    shop.finish().await;
}
