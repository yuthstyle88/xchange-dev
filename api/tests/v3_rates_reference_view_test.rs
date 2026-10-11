//! Ticket #468 (V2-rates, `docs/design/xchange-v3-web.md` §12.16, §12.1, §4.2): the BOT
//! reference panel's view — the one admin poll. Written before the implementation by the lane
//! controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/rates_reference?branch=&since=&fresh=` — a `BranchView` inheriting
//!   `(EntryPath::OwnerRoute, "/api/reference-rates")` (the row the route checks,
//!   `reference_rates.rs:52-53`), filter `{fresh}`, with a version of its own through V0-C's
//!   `own_version` (#475): one server-wide `ReferenceCache` `{version, rows, fetchedAt, error,
//!   checkedAt}` asks the Bank of Thailand only when its `checkedAt` is older than 60 s (or on
//!   `fresh=1`) and raises `version` only when `rows` or `error` change. With `since` equal to it
//!   the answer is `{version, changed: false}` and the BOT is not asked; otherwise `data` =
//!   `{rows, fetchedAt, error}`: the BOT's rows `{code, date, unit, buyingSight, buyingTransfer,
//!   selling}` with every rate a decimal string of four places (v2 prints them through
//!   `toFixed(4)`, `rates-reference.js:3`, and the screen may not, ticket §4 criterion 3) and `unit`
//!   a number (100 for JPY, the note of `rates-reference.js:5`); `fetchedAt` the BOT answer's;
//!   `error` `null`, or the sentence `/api/reference-rates` answers for that failure today
//!   (`reference_rates.rs:80`: the source's refusal; `เซิร์ฟเวอร์ขัดข้อง` for anything else). A
//!   failed fetch keeps the last good rows.
//! - Concurrent stale reads share one fetch (design §12.16 [proposed]: a `tokio::sync::Mutex`
//!   around the refresh).
//!
//! **The seam these tests need** (not in the ticket's text; see the comment on the ticket): the
//! view's `load` and `own_version` get no request, so the source and the clock cannot come from
//! the application data the way `/api/reference-rates` takes them. The view's file therefore
//! exports
//!
//! ```ignore
//! ReferenceCache::new(source: Arc<dyn RateSource>, clock: bot_rate_source::Clock) -> ReferenceCache
//! RatesReferenceView::new(cache: Arc<ReferenceCache>) -> RatesReferenceView
//! ```
//!
//! and its `register` builds the one server-wide cache (the real `BotRateSource::bot()` and
//! `Utc::now`). These tests register their own view, over their own cache, before
//! `bootstrap::configure`, so theirs answers the path (the order `v3_view_test.rs` uses). This
//! file therefore does not compile before the view exists; it is its own test target, so the rest
//! of the suite builds.
//!
//! Sessions are made at the real sign-in doors (`rpc_common`). Needs `TEST_DATABASE_URL`.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::infrastructure::rate::bot_rate_source::Clock;
use xchange::presentation::http::rpc::RpcClock;
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::http::v3::rates_reference::{RatesReferenceView, ReferenceCache};
use xchange::presentation::http::v3::view::ViewEndpoint;

const ORIGIN: &str = "http://desk.test";
const VIEW: &str = "rates_reference";
const MAIN: &str = "MAIN";
const B9: &str = "B9";

const BANK_DOWN: &str = "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท";
const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";
/// Text no answer may carry: what an internal failure said.
const SECRET: &str = "pool timed out at 10.0.0.7:5432";

const DATA_FIELDS: [&str; 3] = ["rows", "fetchedAt", "error"];
const ROW_FIELDS: [&str; 6] = [
    "code",
    "date",
    "unit",
    "buyingSight",
    "buyingTransfer",
    "selling",
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

/// One BOT row as `BotRateSource` builds it: rates as JSON numbers, per one unit.
fn bot_row(code: &str, unit: u64, sight: &str, transfer: &str, selling: &str) -> Value {
    let n = |t: &str| -> Value { serde_json::from_str(t).expect("a JSON number") };
    json!({
        "code": code, "date": "2026-10-06", "unit": unit,
        "buyingSight": n(sight), "buyingTransfer": n(transfer), "selling": n(selling),
        "original": { "buyingSight": sight, "buyingTransfer": transfer, "selling": selling },
    })
}

fn rates(fetched_at: &str, rows: Vec<Value>) -> ReferenceRates {
    ReferenceRates {
        source: "https://www.bot.or.th/th/statistics/exchange-rate.html".to_string(),
        fetched_at: fetched_at.to_string(),
        kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
        rows,
    }
}

/// The day of `tests/fixtures/v2_reference_rates_cases.json` (`answer`), as the BOT gives it.
fn day_a(fetched_at: &str) -> ReferenceRates {
    rates(
        fetched_at,
        vec![
            bot_row("USD", 1, "33.4218", "33.5008", "33.8278"),
            bot_row("GBP", 1, "44.0195", "44.1299", "44.909"),
            bot_row("EUR", 1, "37.3552", "37.4413", "38.0844"),
            bot_row("JPY", 100, "0.2095", "0.2101", "0.2163"),
            bot_row("AUD", 1, "23.0133", "23.0712", "23.7839"),
            bot_row("CNY", 1, "4.9541", "4.9767", "5.0667"),
        ],
    )
}

/// `day_a` as the view answers it: four places, `unit` a number, no `original`.
fn day_a_rows() -> Value {
    let row = |code: &str, unit: u64, s: &str, t: &str, l: &str| {
        json!({ "code": code, "date": "2026-10-06", "unit": unit,
                "buyingSight": s, "buyingTransfer": t, "selling": l })
    };
    json!([
        row("USD", 1, "33.4218", "33.5008", "33.8278"),
        row("GBP", 1, "44.0195", "44.1299", "44.9090"),
        row("EUR", 1, "37.3552", "37.4413", "38.0844"),
        row("JPY", 100, "0.2095", "0.2101", "0.2163"),
        row("AUD", 1, "23.0133", "23.0712", "23.7839"),
        row("CNY", 1, "4.9541", "4.9767", "5.0667"),
    ])
}

/// Another day: USD moved.
fn day_b(fetched_at: &str) -> ReferenceRates {
    let mut b = day_a(fetched_at);
    b.rows[0] = bot_row("USD", 1, "33.5", "33.6", "33.9");
    b
}

// ── the Bank of Thailand behind the view ─────────────────────────────────────

#[derive(Clone)]
enum Next {
    Answer(ReferenceRates),
    Refuse(&'static str),
    Break,
}

/// Counts every fetch; answers what it was told; can take its time.
struct Bot {
    next: Mutex<Next>,
    asked: AtomicU64,
    delay_ms: AtomicU64,
}

impl Bot {
    fn new() -> Bot {
        Bot {
            next: Mutex::new(Next::Answer(day_a("2026-10-06T05:00:00.000Z"))),
            asked: AtomicU64::new(0),
            delay_ms: AtomicU64::new(0),
        }
    }

    fn answer(&self, next: Next) {
        *self.next.lock().expect("the bot's lock") = next;
    }

    fn asked(&self) -> u64 {
        self.asked.load(Ordering::SeqCst)
    }
}

impl RateSource for Bot {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = AppResult<ReferenceRates>> + Send + '_>> {
        Box::pin(async move {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let delay = self.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            let next = self.next.lock().expect("the bot's lock").clone();
            match next {
                Next::Answer(rates) => Ok(rates),
                Next::Refuse(sentence) => Err(AppError::Domain(sentence.to_owned())),
                Next::Break => Err(AppError::Internal(SECRET.to_owned())),
            }
        })
    }
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    bot: Arc<Bot>,
    now: Arc<Mutex<DateTime<Utc>>>,
    t0: DateTime<Utc>,
}

impl Room {
    /// `MAIN` and `B9`, the owner and one employee of every staff role signed in; the view over
    /// a cache of its own, with the test's BOT and clock.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, "Nine").await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        let bot = Arc::new(Bot::new());
        let now = Arc::new(Mutex::new(t0));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || *now.lock().expect("the clock's lock"))
        };
        let source: Arc<dyn RateSource> = bot.clone();
        let cache = Arc::new(ReferenceCache::new(source, clock));
        let view = Arc::new(RatesReferenceView::new(cache));

        let state = desk.state();
        let rpc_clock: Arc<dyn RpcClock> = desk.clock.clone();
        let bank: Arc<dyn RateSource> = desk.bank.clone();
        let app = App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| {
                        ViewEndpoint::register(cfg, view);
                        xchange::bootstrap::configure(cfg, state);
                    })
                    .app_data(actix_web::web::Data::from(rpc_clock))
                    .app_data(actix_web::web::Data::new(bank)),
            )
            .await,
            |t| t.to_request(),
        );
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room {
            desk,
            app,
            cookies,
            bot,
            now,
            t0,
        }
    }

    /// Sets the cache's clock to `seconds` after the start.
    fn at(&self, seconds: i64) {
        *self.now.lock().expect("the clock's lock") = self.t0 + Duration::seconds(seconds);
    }

    fn cookie(&self, who: &str) -> String {
        match who.split_once('+') {
            Some((a, b)) => format!("{}; {}", self.cookies[a], self.cookies[b]),
            None => self.cookies[who].clone(),
        }
    }

    fn call(&self, who: &str, query: &str) -> Call {
        let path = if query.is_empty() {
            format!("/api/v3/views/{VIEW}")
        } else {
            format!("/api/v3/views/{VIEW}?{query}")
        };
        let mut call = Call::post(ORIGIN, &path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin");
        call.method = Method::GET;
        if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        }
    }

    async fn get(&self, who: &str, query: &str) -> Answer {
        self.app.send(self.call(who, query)).await
    }

    /// The view as `who`, answered 200, never cached.
    async fn view(&self, who: &str, query: &str) -> Value {
        let answer = self.get(who, query).await;
        assert_eq!(
            answer.status, 200,
            "{VIEW}?{query} as {who:?}: {:.400}",
            answer.text
        );
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "{VIEW}: never cached"
        );
        assert!(
            !answer.text.contains(SECRET),
            "no internal text: {}",
            answer.text
        );
        answer.json()
    }

    /// An answer that must carry data; `(version, data)`.
    async fn data(&self, who: &str, query: &str) -> (i64, Value) {
        let body = self.view(who, query).await;
        assert_eq!(body["changed"], json!(true), "{VIEW}?{query}: {body}");
        let data = body
            .get("data")
            .cloned()
            .unwrap_or_else(|| panic!("{VIEW}?{query}: data in {body}"));
        assert_eq!(keys(&data), set_of(&DATA_FIELDS), "{data}");
        (body["version"].as_i64().expect("a version"), data)
    }

    async fn version(&self, query: &str) -> i64 {
        self.view("OWNER", query).await["version"]
            .as_i64()
            .expect("a version")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The actor `Policy` sees for each caller key on a path that is not `StaffRpc`.
fn callers() -> Vec<(String, Option<Actor>)> {
    let mut callers: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        ("OWNER".to_string(), Some(Actor::owner("owner"))),
    ];
    for role in Role::STAFF {
        let staff = Actor::staff("employee", role, BranchId::new(MAIN));
        callers.push((role.as_str().to_string(), staff));
        callers.push((
            format!("OWNER+{}", role.as_str()),
            Some(Actor::owner("owner")),
        ));
    }
    callers
}

// ── who may read it ──────────────────────────────────────────────────────────

/// W8: the answer `/api/reference-rates` gives each caller (owner only); a refused caller never
/// makes the BOT be asked.
#[actix_web::test]
async fn rates_reference_answers_exactly_whom_the_reference_rates_route_allows() {
    let room = Room::open().await;
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        let policy = Policy::allow(
            actor.as_ref(),
            EntryPath::OwnerRoute,
            &Action::new("/api/reference-rates"),
        );
        let answer = room.get(&who, "").await;
        let what = format!("{VIEW} as {who:?}");
        match &policy {
            Ok(()) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                assert!(answer.json().get("data").is_some(), "{what}: data");
            }
            Err(e) => {
                refused += 1;
                assert_eq!(answer.status, e.status(), "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["error"]["message"], json!(e.to_string()), "{what}");
                assert!(body.get("data").is_none(), "{what}: no data");
            }
        }
    }
    assert_eq!(allowed, 10, "the owner, and the owner beside each role");
    assert_eq!(refused, 10, "nobody, and each staff role alone");
    assert_eq!(
        room.bot.asked(),
        1,
        "ten allowed reads inside a minute: one BOT call; refusals none"
    );
    room.finish().await;
}

// ── the answer ───────────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_first_read_asks_the_bot_once_and_answers_its_rows_as_four_place_strings() {
    let room = Room::open().await;
    assert_eq!(room.bot.asked(), 0, "nothing is asked before a read");
    let (_, data) = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(room.bot.asked(), 1);
    assert_eq!(data["error"], Value::Null, "{data}");
    assert_eq!(
        data["fetchedAt"],
        json!("2026-10-06T05:00:00.000Z"),
        "{data}"
    );
    for row in data["rows"].as_array().expect("rows") {
        assert_eq!(keys(row), set_of(&ROW_FIELDS), "{row}");
    }
    assert_eq!(
        data["rows"],
        day_a_rows(),
        "the BOT's rows in its order, rates as v2's toFixed(4) prints them"
    );
    room.finish().await;
}

/// Inside 60 s every read is answered from the cache: an equal `since` is exactly
/// `{version, changed: false}`, a read without one gets the rows, whatever the branch — and the
/// BOT is not asked again.
#[actix_web::test]
async fn within_sixty_seconds_every_read_is_answered_from_the_cache() {
    let room = Room::open().await;
    let (v, rows) = room.data("OWNER", "branch=MAIN").await;
    for (seconds, query) in [
        (1, format!("branch=MAIN&since={v}")),
        (30, "branch=MAIN".to_owned()),
        (45, format!("branch=B9&since={v}")),
        (59, "branch=B9".to_owned()),
    ] {
        room.at(seconds);
        let body = room.view("OWNER", &query).await;
        if query.contains("since") {
            assert_eq!(
                body,
                json!({ "version": v, "changed": false }),
                "+{seconds}s {query}"
            );
        } else {
            assert_eq!(body["version"], json!(v), "+{seconds}s {query}: {body}");
            assert_eq!(body["data"], rows, "+{seconds}s {query}");
        }
        assert_eq!(room.bot.asked(), 1, "+{seconds}s {query}: not asked again");
    }
    room.finish().await;
}

/// Older than 60 s, a read asks again — also one carrying `since`, so the shortcut compares
/// with a fresh version — and the version moves only when the rows or the error changed:
/// the same rows with a later `fetchedAt` leave it; a failure moves it once and keeps the last
/// good rows; the same failure again leaves it; the BOT back clears the error.
#[actix_web::test]
async fn a_stale_cache_asks_again_and_the_version_moves_only_when_rows_or_error_change() {
    let room = Room::open().await;
    let (v1, first) = room.data("OWNER", "branch=MAIN").await;

    // the same rows, fetched later
    room.at(61);
    room.bot
        .answer(Next::Answer(day_a("2026-10-06T05:01:01.000Z")));
    assert_eq!(
        room.view("OWNER", &format!("branch=MAIN&since={v1}")).await,
        json!({ "version": v1, "changed": false }),
        "same rows, same error: the version stays"
    );
    assert_eq!(room.bot.asked(), 2, "stale: asked again");

    // the BOT fails: the error moves the version, the rows stay
    room.at(122);
    room.bot.answer(Next::Refuse(BANK_DOWN));
    let body = room.view("OWNER", &format!("branch=MAIN&since={v1}")).await;
    assert_eq!(room.bot.asked(), 3);
    assert_eq!(
        body["changed"],
        json!(true),
        "the error is a change: {body}"
    );
    let v2 = body["version"].as_i64().expect("a version");
    assert!(v2 > v1, "the version rose ({v1} → {v2})");
    assert_eq!(body["data"]["error"], json!(BANK_DOWN), "{body}");
    assert_eq!(
        body["data"]["rows"], first["rows"],
        "a failed fetch keeps the last good rows"
    );

    // the same failure again
    room.at(183);
    assert_eq!(
        room.view("OWNER", &format!("branch=MAIN&since={v2}")).await,
        json!({ "version": v2, "changed": false }),
        "the same error: the version stays"
    );
    assert_eq!(room.bot.asked(), 4);

    // back, with the same rows: the error clears
    room.at(244);
    room.bot
        .answer(Next::Answer(day_a("2026-10-06T05:04:04.000Z")));
    let body = room.view("OWNER", &format!("branch=MAIN&since={v2}")).await;
    assert_eq!(room.bot.asked(), 5);
    assert_eq!(body["changed"], json!(true), "the error cleared: {body}");
    let v3 = body["version"].as_i64().expect("a version");
    assert!(v3 > v2, "the version rose ({v2} → {v3})");
    assert_eq!(body["data"]["error"], Value::Null, "{body}");
    assert_eq!(body["data"]["rows"], first["rows"], "{body}");

    // new rows
    room.at(305);
    room.bot
        .answer(Next::Answer(day_b("2026-10-06T05:05:05.000Z")));
    let body = room.view("OWNER", &format!("branch=MAIN&since={v3}")).await;
    assert_eq!(room.bot.asked(), 6);
    assert_eq!(body["changed"], json!(true), "new rows: {body}");
    let v4 = body["version"].as_i64().expect("a version");
    assert!(v4 > v3, "the version rose ({v3} → {v4})");
    assert_eq!(
        body["data"]["rows"][0],
        json!({ "code": "USD", "date": "2026-10-06", "unit": 1,
                "buyingSight": "33.5000", "buyingTransfer": "33.6000", "selling": "33.9000" }),
        "{body}"
    );
    assert_eq!(
        body["data"]["fetchedAt"],
        json!("2026-10-06T05:05:05.000Z"),
        "the fetchedAt of the answer that produced these rows"
    );
    room.finish().await;
}

/// `โหลดเรท ธปท. ล่าสุด` (`rates-reference.js:7`): `fresh=1` asks the BOT now, however fresh the
/// cache, once; the new rows are then what every read answers, without asking again.
#[actix_web::test]
async fn fresh_asks_the_bot_now_whatever_the_age() {
    let room = Room::open().await;
    let (v1, _) = room.data("OWNER", "branch=MAIN").await;
    room.at(10);
    room.bot
        .answer(Next::Answer(day_b("2026-10-06T05:00:10.000Z")));

    let (_, data) = room.data("OWNER", "branch=MAIN&fresh=1").await;
    assert_eq!(room.bot.asked(), 2, "fresh=1: asked now, once");
    assert_eq!(data["rows"][0]["buyingSight"], json!("33.5000"), "{data}");
    assert_eq!(
        data["fetchedAt"],
        json!("2026-10-06T05:00:10.000Z"),
        "{data}"
    );

    room.at(20);
    let (v2, data) = room.data("OWNER", "branch=MAIN").await;
    assert!(v2 > v1, "the new rows moved the version ({v1} → {v2})");
    assert_eq!(data["rows"][0]["buyingSight"], json!("33.5000"), "{data}");
    let body = room.view("OWNER", &format!("branch=MAIN&since={v1}")).await;
    assert_eq!(
        body["changed"],
        json!(true),
        "the old version is behind: {body}"
    );
    assert_eq!(room.bot.asked(), 2, "not asked again inside the minute");
    room.finish().await;
}

/// Ticket §9 *BOT unavailable*: a first fetch that fails answers no rows and the sentence the
/// route answers today — the source's refusal, or `เซิร์ฟเวอร์ขัดข้อง` for an internal failure,
/// whose own text never leaves the server. The view itself still answers 200.
#[actix_web::test]
async fn a_failed_first_fetch_answers_no_rows_and_the_routes_sentence() {
    let room = Room::open().await;
    room.bot.answer(Next::Refuse(BANK_DOWN));
    let (v1, data) = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(data["rows"], json!([]), "{data}");
    assert_eq!(data["error"], json!(BANK_DOWN), "{data}");

    room.at(61);
    room.bot.answer(Next::Break);
    let body = room.view("OWNER", &format!("branch=MAIN&since={v1}")).await;
    assert_eq!(body["changed"], json!(true), "another error: {body}");
    assert_eq!(body["data"]["error"], json!(SERVER_ERROR), "{body}");
    assert_eq!(body["data"]["rows"], json!([]), "{body}");
    room.finish().await;
}

/// Every open browser together causes at most one BOT call a minute: reads that find the cache
/// stale at the same moment share one fetch and answer the same version and rows.
#[actix_web::test]
async fn concurrent_stale_reads_share_one_fetch() {
    let room = Room::open().await;
    room.bot.delay_ms.store(200, Ordering::SeqCst);
    let call = |q: &str| room.app.send(room.call("OWNER", q));
    let (a, b, c, d, e, f) = tokio::join!(
        call("branch=MAIN"),
        call("branch=MAIN"),
        call("branch=B9"),
        call("branch=MAIN"),
        call("branch=B9"),
        call("branch=MAIN"),
    );
    assert_eq!(room.bot.asked(), 1, "six reads at once: one fetch");
    let answers: Vec<Value> = [a, b, c, d, e, f]
        .iter()
        .map(|x| {
            assert_eq!(x.status, 200, "{:.300}", x.text);
            x.json()
        })
        .collect();
    for x in &answers {
        assert_eq!(x, &answers[0], "the same answer for all six");
    }
    assert_eq!(answers[0]["data"]["rows"], day_a_rows());
    room.finish().await;
}

/// The view's version is the cache's, not the branch's: a write in the branch does not move it.
#[actix_web::test]
async fn a_write_in_the_branch_does_not_move_the_reference_version() {
    let room = Room::open().await;
    let v = room.version("branch=MAIN").await;
    room.desk.clock.advance(Duration::seconds(1));
    // re-expected for RE-7 #604 by the ticket author: `adminSaveRate` is retired (design §9);
    // a rate is put in MAIN the v3 way, by publishing it
    snapshot_common::publish_rate_per_one(&room.desk.db.pool, MAIN, "USD", "30", "40").await;
    room.at(5);
    assert_eq!(
        room.view("OWNER", &format!("branch=MAIN&since={v}")).await,
        json!({ "version": v, "changed": false }),
        "a rate published in MAIN is not a change of the BOT reference"
    );
    assert_eq!(
        room.bot.asked(),
        1,
        "the view's BOT was not asked by the publish"
    );
    room.finish().await;
}

/// Ticket #468: 4-place half-away-from-zero rounding test (e.g. 36.123450 -> "36.1235").
#[actix_web::test]
async fn rates_are_rounded_half_away_from_zero_to_four_places() {
    let room = Room::open().await;
    room.bot.answer(Next::Answer(rates(
        "2026-10-06T05:00:00.000Z",
        vec![bot_row("USD", 1, "36.123450", "36.123449", "36.123451")],
    )));
    let (_, data) = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(data["rows"][0]["buyingSight"], json!("36.1235"));
    assert_eq!(data["rows"][0]["buyingTransfer"], json!("36.1234"));
    assert_eq!(data["rows"][0]["selling"], json!("36.1235"));
    room.finish().await;
}

/// Ticket #468: cache staleness at exactly 60 s boundary (room.at(60) triggers refetch).
#[actix_web::test]
async fn cache_is_stale_at_exactly_sixty_seconds_boundary() {
    let room = Room::open().await;
    let (v1, _) = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(room.bot.asked(), 1);

    // At 59s, cache is still fresh
    room.at(59);
    let body = room.view("OWNER", &format!("branch=MAIN&since={v1}")).await;
    assert_eq!(body, json!({ "version": v1, "changed": false }));
    assert_eq!(room.bot.asked(), 1, "at 59s: not asked again");

    // At exactly 60s, cache is stale and refetches
    room.at(60);
    room.bot
        .answer(Next::Answer(day_b("2026-10-06T05:01:00.000Z")));
    let body = room.view("OWNER", &format!("branch=MAIN&since={v1}")).await;
    assert_eq!(room.bot.asked(), 2, "at exactly 60s: asked again");
    assert_eq!(body["changed"], json!(true));
    room.finish().await;
}

// ── the generated types ──────────────────────────────────────────────────────

fn exported() -> BTreeMap<String, String> {
    let entry = VIEWS
        .iter()
        .find(|v| v.name == VIEW)
        .unwrap_or_else(|| panic!("the view {VIEW} is in the generated table"));
    let dir: PathBuf =
        std::env::temp_dir().join(format!("xc-468r-{}-{}", std::process::id(), next()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    (entry.export_types)(&dir).unwrap_or_else(|e| panic!("export {VIEW}: {e}"));
    let mut files = BTreeMap::new();
    for e in std::fs::read_dir(&dir).expect("read the scratch directory") {
        let e = e.expect("an entry");
        let name = e.file_name().to_string_lossy().into_owned();
        files.insert(
            name,
            std::fs::read_to_string(e.path()).expect("read an exported file"),
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    files
}

fn has_field(ts: &str, field: &str, types: &[&str]) {
    let found = types.iter().any(|ty| {
        ts.contains(&format!("{field}: {ty},")) || ts.contains(&format!("{field}: {ty} "))
    });
    assert!(found, "`{field}: one of {types:?}` in:\n{ts}");
}

#[test]
fn rates_reference_is_in_the_generated_table_with_rates_as_strings_in_its_own_file() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert!(names.contains(&VIEW), "{VIEW} in {names:?}");
    let files = exported();
    let ts = files
        .get("rates_reference.ts")
        .unwrap_or_else(|| panic!("rates_reference.ts in {:?}", files.keys()));
    for field in ["buyingSight", "buyingTransfer", "selling"] {
        has_field(ts, field, &["string", "Rate"]);
    }
    has_field(ts, "unit", &["number"]);
    has_field(ts, "code", &["string"]);
    has_field(ts, "date", &["string"]);
    has_field(ts, "fetchedAt", &["string"]);
    has_field(ts, "error", &["string | null"]);
    assert!(!ts.contains("bigint"), "no bigint on the wire:\n{ts}");
    for name in files.keys() {
        assert!(
            name == "rates_reference.ts" || name == "common.ts",
            "{VIEW} writes only its own file and common.ts: {name}"
        );
    }
}
