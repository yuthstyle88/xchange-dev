//! Up-front tests for ticket CS-4 (docs/design/configurable-shifts.md §2.7, T4, T10, §4 rows W1, W2, W4–W6): the custody
//! window comes from the branch's shift plan, not from `opening_hours`. Written by the ticket author claude/35271802 —
//! do not edit, delete or weaken. Needs CS-1 and CS-3 merged into `feat/configurable-shifts`, and `TEST_DATABASE_URL`.
//!
//! Instants are written in UTC; Bangkok is UTC+7, so 4 Nov 10:00 Bangkok is `2026-11-04T03:00:00Z`.

#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use save_common::*;
use serde_json::json;
use xc_shift::domain::custody::repositories::custody_ports::CustodyPorts;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

fn ct(iso: &str) -> CustodyTime {
    CustodyTime::from_epoch_millis(instant(iso).timestamp_millis())
}

fn open(from: &str, to: &str) -> ServiceWindow {
    ServiceWindow::Open {
        open: ct(from),
        close: ct(to),
    }
}

async fn window(shop: &Shop, now: &str) -> ServiceWindow {
    let mut tx = shop.pool.begin().await.expect("begin");
    let got = Arc::new(PgCustodyPorts)
        .service_window(&mut tx, &branch(MAIN), ct(now))
        .await
        .expect("service_window");
    tx.rollback().await.expect("rollback");
    got
}

/// Replaces MAIN's live plan with `rows` of `(name, start, end)`.
async fn set_plan(shop: &Shop, rows: &[(&str, &str, &str)]) {
    sqlx::query(
        "UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1 AND retired_at IS NULL",
    )
    .bind(MAIN)
    .execute(&shop.pool)
    .await
    .expect("retire MAIN's plan");
    for (i, (name, start, end)) in rows.iter().enumerate() {
        sqlx::query(
            "INSERT INTO shift_templates (organization_id, branch_id, name, starts_at, ends_at, position) \
             SELECT organization_id, id, $2, $3::time, $4::time, $5 FROM branches WHERE id = $1",
        )
        .bind(MAIN)
        .bind(name)
        .bind(start)
        .bind(end)
        .bind(i as i16)
        .execute(&shop.pool)
        .await
        .expect("insert a template");
    }
}

/// An overlapping 24-hour plan: A 06–15, B 14–23, C 22–07 (cutover 06:00).
async fn overlapping(shop: &Shop) {
    set_plan(
        shop,
        &[
            ("A", "06:00", "15:00"),
            ("B", "14:00", "23:00"),
            ("C", "22:00", "07:00"),
        ],
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn w1_w2_the_window_is_the_business_days_24_hours_joined_with_the_next() {
    let shop = Shop::start().await;
    overlapping(&shop).await;
    // 4 Nov 06:00 → 6 Nov 06:00 Bangkok: the 4 Nov business day joined with the 5 Nov one (T10).
    let joined = open("2026-11-03T23:00:00.000Z", "2026-11-05T23:00:00.000Z");
    // W1: 4 Nov 11:00 Bangkok
    assert_eq!(window(&shop, "2026-11-04T04:00:00.000Z").await, joined);
    // W2: 5 Nov 02:00 Bangkok still belongs to the 4 Nov business day
    assert_eq!(window(&shop, "2026-11-04T19:00:00.000Z").await, joined);
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn w4_a_branch_without_a_live_template_has_no_window() {
    let shop = Shop::start().await;
    set_plan(&shop, &[]).await;
    assert_eq!(
        window(&shop, "2026-11-04T04:00:00.000Z").await,
        ServiceWindow::NotSet
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn w5_opening_hours_no_longer_gate_custody() {
    let shop = Shop::start().await;
    overlapping(&shop).await;
    let day = json!([{ "open": "12:00", "close": "18:00" }]);
    sqlx::query("UPDATE branches SET opening_hours = $2 WHERE id = $1")
        .bind(MAIN)
        .bind(json!({"mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": day}))
        .execute(&shop.pool)
        .await
        .unwrap();
    // 4 Nov 20:00 Bangkok: outside 12–18, inside the plan's business day
    let now = "2026-11-04T13:00:00.000Z";
    let w = window(&shop, now).await;
    assert_eq!(
        w,
        open("2026-11-03T23:00:00.000Z", "2026-11-05T23:00:00.000Z")
    );
    assert!(w.allows_begin(ct(now)));
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn w6_a_twenty_four_hour_plan_joins_days_so_selling_never_pauses() {
    let shop = Shop::start().await;
    // MAIN keeps the migration's defaults: A 06–14, B 14–22, C 22–06.
    // 5 Nov 05:55 Bangkok: the 4 Nov span (06:00 → 06:00) joined with the 5 Nov span.
    let now = "2026-11-04T22:55:00.000Z";
    let w = window(&shop, now).await;
    assert_eq!(
        w,
        open("2026-11-03T23:00:00.000Z", "2026-11-05T23:00:00.000Z")
    );
    assert!(
        w.allows_begin(ct(now)),
        "no 05:50–06:00 pause (owner Q7 = A)"
    );
    // 5 Nov 06:00 Bangkok: the 5 Nov span joined with the 6 Nov span.
    assert_eq!(
        window(&shop, "2026-11-04T23:00:00.000Z").await,
        open("2026-11-04T23:00:00.000Z", "2026-11-06T23:00:00.000Z")
    );
    shop.finish().await;
}
