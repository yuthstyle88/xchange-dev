//! R2 / F29 (#564): the kiosk and the customer's queue are open only inside the branch's
//! opening hours.
//!
//! Written before the implementation by the ticket author (claude/33691c06). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Owner ruling R2 (A) on #564, issuecomment-6673699296 (2026-11-06 07:56 ICT):
//! "branches.opening_hours governs everything a customer sees, and the kiosk must also be inside
//! those hours." The symptom (#564 issuecomment-6657891388, F29): the kiosk answered
//! `open: true` at 19:00 while the customer hours and the booking slots stopped at 18:00.
//!
//! Pinned here:
//! - the kiosk's `rates` answers `open: true` only when a shift is OPEN **and** the request's
//!   `now` (the route's `RpcClock`, read in the branch's own timezone) is inside today's
//!   opening-hours window, or inside yesterday's window when that one runs past midnight;
//!   a window is `[open, close)`;
//! - NULL `opening_hours` is closed for the kiosk, as it already is for booking slots
//!   (`slot_generator.rs`); a weekday missing from `opening_hours` is closed that day;
//! - outside the hours `quote` is refused with `KioskError::CounterClosed` and `confirm` with
//!   `KioskError::RateChangedOrClosed`, the same refusals a branch without an open shift gets;
//! - the public branch list's `queueOpen` is the stored `branch_service_status.queue_open` **and**
//!   inside the hours (NULL hours ⇒ `false`), and its ETag changes when that answer changes, so a
//!   `304` never hides the queue closing.
//!
//! Staff paths (the staff floor, shifts, selling at the desk, the queue machine issuing tickets)
//! are not gated by the hours and are not touched here.
//!
//! The harness is `kiosk_http_common::Counter`: `MAIN` (Asia/Bangkok) with an open shift, an open
//! queue and a published dollar, the whole route table, and the test's clock. Each test writes
//! `MAIN`'s `opening_hours` itself.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeMap;

use actix_web::http::Method;
use chrono::{DateTime, Utc};
use kiosk_http_common::Counter;
use rpc_common::{Answer, Call, MAIN};
use serde_json::{Value, json};

/// v2's sentence for `KioskError::CounterClosed`.
const COUNTER_CLOSED: &str = "Counter is closed. Please contact our staff.";
/// v2's sentence for `KioskError::RateChangedOrClosed`.
const RATE_CHANGED_OR_CLOSED: &str = "Rate changed or counter closed. Please start again.";

fn at(instant: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(instant)
        .unwrap_or_else(|e| panic!("an RFC 3339 instant {instant}: {e}"))
        .with_timezone(&Utc)
}

/// The same window on every day of the week.
fn every_day(open: &str, close: &str) -> Value {
    let day = json!([{ "open": open, "close": close }]);
    json!({
        "mon": day, "tue": day, "wed": day, "thu": day,
        "fri": day, "sat": day, "sun": day,
    })
}

async fn set_hours(counter: &Counter, hours: Option<Value>) {
    sqlx::query("UPDATE branches SET opening_hours = $1 WHERE id = $2")
        .bind(hours)
        .bind(MAIN)
        .execute(&counter.desk.db.pool)
        .await
        .expect("write MAIN's opening hours");
}

async fn set_timezone(counter: &Counter, timezone: &str) {
    sqlx::query("UPDATE branches SET timezone = $1 WHERE id = $2")
        .bind(timezone)
        .bind(MAIN)
        .execute(&counter.desk.db.pool)
        .await
        .expect("write MAIN's timezone");
}

/// The kiosk's `rates` answer `open` at `instant`.
async fn kiosk_open_at(counter: &Counter, instant: &str) -> Value {
    counter.desk.clock.set(at(instant));
    let answer = counter.ask(None, "rates", json!({})).await;
    assert_eq!(answer.status, 200, "rates at {instant}: {}", answer.text);
    answer.json()["open"].clone()
}

async fn quote_dollars(counter: &Counter) -> Answer {
    counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        )
        .await
}

async fn requests(counter: &Counter) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kiosk_requests")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("count kiosk_requests")
}

/// `GET /api/public/orgs/XC/branches` through the counter's route table (and so its clock).
async fn public_branches(counter: &Counter, if_none_match: Option<&str>) -> Answer {
    let mut headers = BTreeMap::new();
    if let Some(etag) = if_none_match {
        headers.insert("if-none-match".to_string(), etag.to_string());
    }
    let call = Call {
        method: Method::GET,
        host: "desk.test".to_string(),
        path: "/api/public/orgs/XC/branches".to_string(),
        headers,
        body: Vec::new(),
    };
    counter.app.send(call).await
}

/// `MAIN`'s `queueOpen` in a 200 answer of the public branch list.
fn main_queue_open(answer: &Answer) -> Value {
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    answer.json()["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("a branches list: {:.300}", answer.text))
        .iter()
        .find(|b| b["id"] == MAIN)
        .unwrap_or_else(|| panic!("MAIN is listed: {:.300}", answer.text))["queueOpen"]
        .clone()
}

// ── the kiosk's `rates` ──────────────────────────────────────────────────────

/// F29 itself: hours 09:00–18:00 (Asia/Bangkok), a shift open all along. 12:00 and 09:00 sharp are
/// open; 18:00 sharp and 19:00 are not.
#[actix_web::test]
async fn f29_rates_is_open_only_inside_todays_window() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, Some(every_day("09:00", "18:00"))).await;

    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T05:00:00Z").await,
        json!(true),
        "12:00 ICT, inside 09:00-18:00, a shift open"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T02:00:00Z").await,
        json!(true),
        "09:00 ICT sharp: a window opens at its open time"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T12:00:00Z").await,
        json!(false),
        "19:00 ICT, after 18:00, the shift still open (F29)"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T11:00:00Z").await,
        json!(false),
        "18:00 ICT sharp: a window is closed at its close time"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T01:59:00Z").await,
        json!(false),
        "08:59 ICT, before 09:00"
    );
    counter.finish().await;
}

/// NULL `opening_hours` (hours not set) is closed for the kiosk, as for booking slots.
#[actix_web::test]
async fn f29_rates_is_closed_when_the_branch_has_no_hours() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, None).await;

    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T05:00:00Z").await,
        json!(false),
        "NULL opening_hours at 12:00 ICT with a shift open"
    );
    counter.finish().await;
}

/// A weekday missing from `opening_hours` is closed all that day.
#[actix_web::test]
async fn f29_rates_is_closed_on_a_day_without_a_window() {
    let counter = Counter::open(&[]).await;
    set_hours(
        &counter,
        Some(json!({ "mon": [{ "open": "09:00", "close": "18:00" }] })),
    )
    .await;

    assert_eq!(
        kiosk_open_at(&counter, "2026-10-05T05:00:00Z").await,
        json!(true),
        "Monday 12:00 ICT, inside Monday's window"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T05:00:00Z").await,
        json!(false),
        "Tuesday 12:00 ICT: Tuesday has no window"
    );
    counter.finish().await;
}

/// `00:00`–`00:00` is a whole day (a close at or before the open ends the next day,
/// `BranchHours::window`): open at every instant. Harnesses that keep a branch open whatever their
/// clock rely on it (`xc_testdb::open_all_day`).
#[actix_web::test]
async fn f29_midnight_to_midnight_is_open_all_day() {
    let counter = Counter::open(&[]).await;
    // Every case after this one is open today too; this first one is not, so the test is red today.
    set_hours(&counter, None).await;
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T05:00:00Z").await,
        json!(false),
        "NULL hours first: closed"
    );
    set_hours(&counter, Some(every_day("00:00", "00:00"))).await;
    for instant in [
        "2026-10-05T17:00:00Z", // Tuesday 00:00 ICT
        "2026-10-06T05:00:00Z", // Tuesday 12:00 ICT
        "2026-10-06T16:59:59Z", // Tuesday 23:59:59 ICT
    ] {
        assert_eq!(
            kiosk_open_at(&counter, instant).await,
            json!(true),
            "00:00-00:00 every day, at {instant}"
        );
    }
    counter.finish().await;
}

/// A window that runs past midnight (18:00–02:00) keeps the kiosk open after midnight, by
/// yesterday's window, and closed between 02:00 and 18:00.
#[actix_web::test]
async fn f29_an_overnight_window_is_open_after_midnight() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, Some(every_day("18:00", "02:00"))).await;

    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T18:00:00Z").await,
        json!(true),
        "Wednesday 01:00 ICT, inside Tuesday's 18:00-02:00 window"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T13:00:00Z").await,
        json!(true),
        "Tuesday 20:00 ICT, inside Tuesday's window"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T19:30:00Z").await,
        json!(false),
        "Wednesday 02:30 ICT, after Tuesday's window closed"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-07T05:00:00Z").await,
        json!(false),
        "Wednesday 12:00 ICT, before Wednesday's window opens"
    );
    counter.finish().await;
}

/// The window is read in the branch's own timezone, not in Bangkok's.
#[actix_web::test]
async fn f29_the_window_is_read_in_the_branch_timezone() {
    let counter = Counter::open(&[]).await;
    set_timezone(&counter, "Asia/Tokyo").await;
    set_hours(&counter, Some(every_day("09:00", "18:00"))).await;

    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T09:30:00Z").await,
        json!(false),
        "18:30 in Tokyo (16:30 in Bangkok): after the branch's 18:00"
    );
    assert_eq!(
        kiosk_open_at(&counter, "2026-10-06T00:30:00Z").await,
        json!(true),
        "09:30 in Tokyo (07:30 in Bangkok): inside the branch's window"
    );
    counter.finish().await;
}

// ── quote and confirm ────────────────────────────────────────────────────────

/// After closing, `quote` is refused as the counter closed, and nothing is stored.
#[actix_web::test]
async fn f29_quote_after_closing_is_refused_as_counter_closed() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, Some(every_day("09:00", "18:00"))).await;
    let before = requests(&counter).await;

    counter.desk.clock.set(at("2026-10-06T12:00:00Z"));
    let late = quote_dollars(&counter).await;
    late.assert_refused(
        400,
        COUNTER_CLOSED,
        "a quote at 19:00 ICT, hours 09:00-18:00",
    );
    assert_eq!(
        requests(&counter).await,
        before,
        "a refused quote stores no request"
    );

    counter.desk.clock.set(at("2026-10-06T05:00:00Z"));
    let inside = quote_dollars(&counter).await;
    assert_eq!(
        inside.status, 200,
        "the same quote at 12:00 ICT: {}",
        inside.text
    );
    counter.finish().await;
}

/// NULL hours: `quote` is refused as the counter closed.
#[actix_web::test]
async fn f29_quote_without_hours_is_refused_as_counter_closed() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, None).await;

    counter.desk.clock.set(at("2026-10-06T05:00:00Z"));
    let answer = quote_dollars(&counter).await;
    answer.assert_refused(400, COUNTER_CLOSED, "a quote with NULL opening_hours");
    counter.finish().await;
}

/// A quote made at 17:59:58 and confirmed at 18:00:02 (by the kiosk's signed-in front staff, as
/// `Counter::waiting_request` confirms) is refused at confirm, as a confirm on a closed counter is.
#[actix_web::test]
async fn f29_confirm_after_closing_is_refused() {
    let counter = Counter::open(&[kiosk_http_common::person("FR1", "FRONT", 0x29)]).await;
    set_hours(&counter, Some(every_day("09:00", "18:00"))).await;
    let id = "7e570000-0000-4000-9000-0000000f2901";
    counter.hands.ids.push(id);

    counter.desk.clock.set(at("2026-10-06T10:59:58Z"));
    let quote = quote_dollars(&counter).await;
    assert_eq!(quote.status, 200, "quote at 17:59:58 ICT: {}", quote.text);
    assert_eq!(quote.json()["id"], id, "the route used the id it was given");

    counter.desk.clock.set(at("2026-10-06T11:00:02Z"));
    let confirmed = counter
        .ask(Some("FR1"), "confirm", json!({ "id": id }))
        .await;
    confirmed.assert_refused(
        400,
        RATE_CHANGED_OR_CLOSED,
        "confirm at 18:00:02 ICT, hours 09:00-18:00",
    );
    counter.finish().await;
}

// ── the public branch list's `queueOpen` ─────────────────────────────────────

/// `queueOpen` is the stored open queue only inside the hours.
#[actix_web::test]
async fn f29_public_queue_open_is_false_outside_hours() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, Some(every_day("09:00", "18:00"))).await;

    counter.desk.clock.set(at("2026-10-06T05:00:00Z"));
    assert_eq!(
        main_queue_open(&public_branches(&counter, None).await),
        json!(true),
        "12:00 ICT, MAIN's queue open"
    );
    counter.desk.clock.set(at("2026-10-06T12:00:00Z"));
    assert_eq!(
        main_queue_open(&public_branches(&counter, None).await),
        json!(false),
        "19:00 ICT, MAIN's queue still stored open, after 18:00"
    );
    counter.finish().await;
}

/// NULL hours: `queueOpen` is `false` whatever is stored.
#[actix_web::test]
async fn f29_public_queue_open_is_false_without_hours() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, None).await;

    counter.desk.clock.set(at("2026-10-06T05:00:00Z"));
    assert_eq!(
        main_queue_open(&public_branches(&counter, None).await),
        json!(false),
        "NULL opening_hours, MAIN's queue stored open"
    );
    counter.finish().await;
}

/// The list's ETag follows `queueOpen`: the ETag taken at 12:00 does not answer `304` at 19:00.
#[actix_web::test]
async fn f29_public_list_etag_changes_when_the_hours_close_the_queue() {
    let counter = Counter::open(&[]).await;
    set_hours(&counter, Some(every_day("09:00", "18:00"))).await;

    counter.desk.clock.set(at("2026-10-06T05:00:00Z"));
    let noon = public_branches(&counter, None).await;
    assert_eq!(main_queue_open(&noon), json!(true), "12:00 ICT");
    let etag = noon
        .header("etag")
        .unwrap_or_else(|| panic!("an ETag: {:?}", noon.headers))
        .to_string();

    let still_noon = public_branches(&counter, Some(&etag)).await;
    assert_eq!(
        still_noon.status, 304,
        "unchanged at 12:00 ICT: {:.300}",
        still_noon.text
    );

    counter.desk.clock.set(at("2026-10-06T12:00:00Z"));
    let evening = public_branches(&counter, Some(&etag)).await;
    assert_eq!(
        main_queue_open(&evening),
        json!(false),
        "19:00 ICT with the noon ETag: a full answer whose queue is closed, not a 304"
    );
    counter.finish().await;
}
