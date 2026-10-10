//! Ticket #468 (V2-rates, `docs/design/xchange-v3-web.md` §4.2, §5.1, §12.1, §12.14): the two
//! views the admin rate menu reads. Written before the implementation by the lane controller
//! (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/admin_rates?branch=&since=` — a `BranchView` inheriting
//!   `(EntryPath::Rpc, "adminSnapshot")`, `NoFilter`, bounded by the currencies of one branch.
//!   `data` = `{updated, rates}`:
//!   - `updated`: the time of the branch's newest rate row whatever its currency — the snapshot's
//!     `updated` (`snapshot.rs:98-99`), which v2 prints as `แก้ไขล่าสุด …` (`admin-html:68`) — or
//!     `""` when the branch has none;
//!   - `rates`: the snapshot's `rates` (`snapshot.rs:96`, `v2_json.rs:296`), same currencies in the
//!     same order, each `{code, name, symbol, buy, sell, spread, updated, partnerName, partnerCost,
//!     partnerSpread}`. `buy`, `sell` and `partnerCost` are the stored rates; `spread` is
//!     `sell − buy` (the `ส่วนต่างเรท` column, `admin-html:68`) and `partnerSpread` is
//!     `sell − partnerCost` (the `ส่วนต่างขาย − ต้นทุน` column, `rates-reference.js:7`), both
//!     computed by the server (W7). Every rate is a decimal string with four places — v2 shows
//!     every rate through `toFixed(4)` (`admin-html:68`, `rates-reference.js:3`) and the screen
//!     may not (ticket §4 criterion 3) — or `null` where v2 shows `ยังไม่ตั้ง` / `—` (ticket §9).
//!     `symbol` is `""` for a currency without one; `updated` is the row's ISO time, exactly the
//!     text `adminSaveRate` expects back as `previousUpdated` (`rate/service.rs` step 12), or `""`.
//!     `partnerName` / `partnerCost` / `partnerSpread` are not in the ticket's §7 list: the panel
//!     (`rates-reference.js:7`) and the edit form (`:10`), rows R2 and R3, show them — see the
//!     comment on the ticket.
//! - `GET /api/v3/views/admin_rate_history?branch=&before=&limit=` — a `PagedView` inheriting
//!   `(EntryPath::Rpc, "adminSnapshot")`, `NoFilter`, newest first (design §12.14): the snapshot's
//!   `rateHistory` (`v2_json.rs:335`) as rows `{id, code, buy, sell, updated, actor, partnerName,
//!   partnerCost, botReference}`, `id` a string the next page's `before` takes. `botReference` is
//!   the copy of the BOT reference stored with the row (`rates-reference.js:8`), as
//!   `{fetchedAt, rows: [{code, date, unit, buyingSight, buyingTransfer, selling}]}` with each rate
//!   a four-place decimal string and `unit` a number, or `null` when the row has none.
//! - §4.2: every writer of what `admin_rates` reads goes through `BranchTx` — `adminSaveRate`
//!   (`rate/service.rs`), the kiosk page's `configure` (which saves through the same service) and
//!   `adminCreateBranch` (`branch/service.rs`, under `MAIN`'s lock) — one write each, then the
//!   same `since` answers `changed: true`.
//! - Ticket §8: a save as `RATES` or the owner is saved and the view re-read shows it; a save as
//!   any other caller gets the policy's refusal and the view does not move (W8).
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`); rates are saved through the real
//! `/api/rpc` `adminSaveRate`, with the test's Bank of Thailand behind it.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing skips.
//!
//! Amended by the ticket author of #603 (RE-6, claude/e5ab6c): the kiosk `configure` saves no
//! rate (item 1) and switches on only a published currency (item 2). In
//! `every_writer_of_the_rates_moves_the_version_it_answers` the euro is published first and the
//! `configure` writer still moves the version, but writes no rate the view would show.

mod common;
mod compat_common;
mod publication_seed;
mod rpc_common;
mod snapshot_common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xc_rate::domain::rate::entities::rate_source::ReferenceRates;
use xchange::presentation::http::v3::VIEWS;

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";
const RATES: &str = "admin_rates";
const HISTORY: &str = "admin_rate_history";

const MAIN: &str = "MAIN";
const B9: &str = "B9";

const DATA_FIELDS: [&str; 2] = ["updated", "rates"];
const RATE_FIELDS: [&str; 10] = [
    "code",
    "name",
    "symbol",
    "buy",
    "sell",
    "spread",
    "updated",
    "partnerName",
    "partnerCost",
    "partnerSpread",
];
const HISTORY_FIELDS: [&str; 9] = [
    "id",
    "code",
    "buy",
    "sell",
    "updated",
    "actor",
    "partnerName",
    "partnerCost",
    "botReference",
];
const BOT_ROW_FIELDS: [&str; 6] = [
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

/// A request id in v2's shape, different on every call.
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
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

/// A rate on the wire: a decimal string with exactly four places (v2's `toFixed(4)`), read
/// exactly, never through a float.
fn rate4(value: &Value, what: &str) -> Decimal {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{what}: a decimal string on the wire (W7), got {value}"));
    let (whole, places) = text
        .split_once('.')
        .unwrap_or_else(|| panic!("{what}: four places, got {text:?}"));
    assert!(
        !whole.is_empty()
            && whole
                .trim_start_matches('-')
                .chars()
                .all(|c| c.is_ascii_digit())
            && places.len() == 4
            && places.chars().all(|c| c.is_ascii_digit()),
        "{what}: a decimal string with exactly four places, got {text:?}"
    );
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{what}: {text:?}: {e}"))
}

/// A snapshot figure (a JSON number, v2's) as a decimal.
fn number(value: &Value, what: &str) -> Decimal {
    assert!(
        value.is_number(),
        "{what}: the snapshot's number, got {value}"
    );
    Decimal::from_str(&value.to_string()).unwrap_or_else(|e| panic!("{what}: {value}: {e}"))
}

/// A GET as the screen sends it: no body, no marker header.
fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn view_path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
    }
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

fn reference(fetched_at: &str, rows: Vec<Value>) -> ReferenceRates {
    ReferenceRates {
        source: "https://www.bot.or.th/th/statistics/exchange-rate.html".to_string(),
        fetched_at: fetched_at.to_string(),
        kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
        rows,
    }
}

/// What the screen sends for a partner: `None` = no partner (name `""`, cost `null`).
type Partner = Option<(&'static str, &'static str)>;

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl Room {
    /// `MAIN` (from the migration) and `B9`; the owner and one employee of every staff role in
    /// `MAIN`, all signed in.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, "Nine").await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        Room {
            desk,
            app,
            cookies,
            t0,
        }
    }

    /// Moves the test's clock forward; every call is later than the one before.
    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    fn cookie(&self, who: &str) -> String {
        match who.split_once('+') {
            Some((a, b)) => format!("{}; {}", self.cookies[a], self.cookies[b]),
            None => self.cookies[who].clone(),
        }
    }

    /// `GET path` as `who` (`""` = nobody; `OWNER+FRONT` = both cookies in one browser).
    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    /// A view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, who: &str, view: &str, query: &str) -> Value {
        let path = view_path(view, query);
        let answer = self.get(who, &path).await;
        assert_eq!(
            answer.status, 200,
            "GET {path} as {who:?}: {:.400}",
            answer.text
        );
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "GET {path}: never cached"
        );
        answer.json()
    }

    /// `data` of an answer that must be `changed: true`.
    async fn data(&self, who: &str, view: &str, query: &str) -> Value {
        let body = self.view(who, view, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {}: {body}",
            view_path(view, query)
        );
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: data in {body}", view_path(view, query)))
    }

    async fn rates(&self, who: &str, query: &str) -> Value {
        self.data(who, RATES, query).await
    }

    /// One page of the history.
    async fn history(&self, who: &str, query: &str) -> Vec<Value> {
        self.data(who, HISTORY, query)
            .await
            .as_array()
            .unwrap_or_else(|| panic!("{HISTORY}'s data is a list of rows"))
            .clone()
    }

    async fn post(&self, who: &str, path: &str, body: Value) -> Answer {
        self.tick();
        let call = Call::post(ORIGIN, path, body.to_string()).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    async fn rpc(&self, who: &str, action: &str, payload: Value) -> Answer {
        self.tick();
        let call = Call::action(ORIGIN, RPC, action, payload).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    async fn snapshot(&self, branch: &str) -> Value {
        let answer = self
            .rpc("OWNER", "adminSnapshot", json!({ "branchId": branch }))
            .await;
        assert_eq!(answer.status, 200, "adminSnapshot: {:.300}", answer.text);
        answer.json()
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    /// The `updated` v2's form sends back as `previousUpdated`: the newest row of `code`, as
    /// the snapshot prints it, or `""`.
    async fn previous(&self, branch: &str, code: &str) -> String {
        let snapshot = self.snapshot(branch).await;
        snapshot["rates"]
            .as_array()
            .expect("the snapshot's rates")
            .iter()
            .find(|r| r["code"] == json!(code))
            .and_then(|r| r["updated"].as_str())
            .unwrap_or("")
            .to_owned()
    }

    /// `adminSaveRate` as `who`, with v2's payload (`admin-html:134`, `rates-reference.js:10`).
    async fn save_rate_as(
        &self,
        who: &str,
        branch: &str,
        code: &str,
        (buy, sell): (&str, &str),
        partner: Partner,
        previous: &str,
    ) -> Answer {
        let (name, cost) = match partner {
            Some((name, cost)) => (json!(name), json!(cost)),
            None => (json!(""), Value::Null),
        };
        self.rpc(
            who,
            "adminSaveRate",
            json!({ "branchId": branch, "requestId": request_id(), "code": code,
                    "buy": buy, "sell": sell, "partnerName": name, "partnerCost": cost,
                    "previousUpdated": previous }),
        )
        .await
    }

    /// The owner saves a rate; it must be saved.
    async fn save_rate(&self, branch: &str, code: &str, buy: &str, sell: &str, partner: Partner) {
        let previous = self.previous(branch, code).await;
        let answer = self
            .save_rate_as("OWNER", branch, code, (buy, sell), partner, &previous)
            .await;
        assert_eq!(
            answer.status, 200,
            "adminSaveRate {code} {buy}/{sell} in {branch}: {:.300}",
            answer.text
        );
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The actor `Policy` sees for each caller key of [`Room`] on a path that is not `StaffRpc`.
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

fn rate_of<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["rates"]
        .as_array()
        .unwrap_or_else(|| panic!("rates is a list: {data}"))
        .iter()
        .find(|r| r["code"] == json!(code))
        .unwrap_or_else(|| panic!("{code} in rates"))
}

// ── who may read them ────────────────────────────────────────────────────────

#[actix_web::test]
async fn both_views_answer_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    for view in [RATES, HISTORY] {
        let mut allowed = 0;
        let mut refused = 0;
        for (who, actor) in callers() {
            let policy = Policy::allow(
                actor.as_ref(),
                EntryPath::Rpc,
                &Action::new("adminSnapshot"),
            );
            let answer = room.get(&who, &view_path(view, "")).await;
            let what = format!("{view} as {who:?}");
            match &policy {
                Ok(()) => {
                    allowed += 1;
                    assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                    let body = answer.json();
                    assert_eq!(body["changed"], json!(true), "{what}: {body}");
                    assert!(body.get("data").is_some(), "{what}: data in {body}");
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
        assert_eq!(
            allowed, 19,
            "{view}: the owner, the nine roles, the owner beside each"
        );
        assert_eq!(refused, 1, "{view}: nobody signed in");
    }
    room.finish().await;
}

// ── admin_rates: the table ───────────────────────────────────────────────────

/// The same currencies, in the same order, with the same stored figures as the snapshot the
/// v2 table reads (`admin-html:68`) — and nothing for the screen to compute.
#[actix_web::test]
async fn admin_rates_is_the_snapshots_rates_with_the_spreads_computed_by_the_server() {
    let room = Room::open().await;
    room.save_rate(MAIN, "USD", "30", "40", None).await;
    room.save_rate(MAIN, "USD", "33.25", "34.1", Some(("SuperRich", "33.9")))
        .await;
    room.save_rate(MAIN, "EUR", "37.3552", "38.0844", Some(("Vasu", "38.5")))
        .await;
    room.save_rate(MAIN, "JPY", "0.2095", "0.2163", None).await;

    let snapshot = room.snapshot(MAIN).await;
    let data = room.rates("OWNER", "branch=MAIN").await;
    assert_eq!(keys(&data), set_of(&DATA_FIELDS), "{data}");
    assert_eq!(
        data["updated"], snapshot["updated"],
        "the newest rate row of the branch, as the snapshot's `updated`"
    );

    let ours = data["rates"].as_array().expect("rates is a list");
    let theirs = snapshot["rates"].as_array().expect("the snapshot's rates");
    assert_eq!(
        ours.iter().map(|r| r["code"].clone()).collect::<Vec<_>>(),
        theirs.iter().map(|r| r["code"].clone()).collect::<Vec<_>>(),
        "the snapshot's currencies, in its order"
    );
    for (ours, theirs) in ours.iter().zip(theirs) {
        let code = ours["code"].as_str().expect("a code");
        assert_eq!(keys(ours), set_of(&RATE_FIELDS), "{code}: {ours}");
        assert_eq!(ours["name"], theirs["name"], "{code}: name");
        assert_eq!(
            ours["symbol"],
            theirs.get("symbol").cloned().unwrap_or(json!("")),
            "{code}: symbol, \"\" when none"
        );
        assert_eq!(ours["updated"], theirs["updated"], "{code}: updated");
        assert_eq!(
            ours["partnerName"], theirs["partnerName"],
            "{code}: partnerName"
        );
        for field in ["buy", "sell", "partnerCost"] {
            if theirs[field].is_null() {
                assert_eq!(ours[field], Value::Null, "{code}.{field}: unset = null");
            } else {
                assert_eq!(
                    rate4(&ours[field], &format!("{code}.{field}")),
                    number(&theirs[field], &format!("snapshot {code}.{field}")),
                    "{code}.{field}"
                );
            }
        }
        if theirs["buy"].is_null() {
            assert_eq!(ours["spread"], Value::Null, "{code}: no rate, no spread");
        } else {
            assert_eq!(
                rate4(&ours["spread"], &format!("{code}.spread")),
                number(&theirs["sell"], "sell") - number(&theirs["buy"], "buy"),
                "{code}: spread = sell − buy"
            );
        }
        if theirs["partnerCost"].is_null() {
            assert_eq!(
                ours["partnerSpread"],
                Value::Null,
                "{code}: no partner cost, no margin"
            );
        } else {
            assert_eq!(
                rate4(&ours["partnerSpread"], &format!("{code}.partnerSpread")),
                number(&theirs["sell"], "sell") - number(&theirs["partnerCost"], "cost"),
                "{code}: partnerSpread = sell − partnerCost"
            );
        }
    }

    // the figures, exactly as the wire carries them
    let usd = rate_of(&data, "USD");
    assert_eq!(usd["buy"], json!("33.2500"), "{usd}");
    assert_eq!(usd["sell"], json!("34.1000"), "{usd}");
    assert_eq!(usd["spread"], json!("0.8500"), "{usd}");
    assert_eq!(usd["partnerName"], json!("SuperRich"), "{usd}");
    assert_eq!(usd["partnerCost"], json!("33.9000"), "{usd}");
    assert_eq!(usd["partnerSpread"], json!("0.2000"), "{usd}");
    let eur = rate_of(&data, "EUR");
    assert_eq!(eur["spread"], json!("0.7292"), "{eur}");
    assert_eq!(
        eur["partnerSpread"],
        json!("-0.4156"),
        "a cost above the selling rate: a negative margin, as v2 prints it: {eur}"
    );
    let jpy = rate_of(&data, "JPY");
    assert_eq!(jpy["buy"], json!("0.2095"), "{jpy}");
    assert_eq!(jpy["spread"], json!("0.0068"), "{jpy}");
    assert_eq!(jpy["partnerName"], json!(""), "{jpy}");
    assert_eq!(jpy["partnerCost"], Value::Null, "{jpy}");
    room.finish().await;
}

/// Ticket §9: a currency with no rate is answered `null` (v2's `ยังไม่ตั้ง` / `—`), never `"0"`;
/// a branch with no rate at all answers `updated: ""`.
#[actix_web::test]
async fn a_currency_with_no_rate_is_null_and_a_branch_without_rates_has_no_updated() {
    let room = Room::open().await;
    let data = room.rates("OWNER", "branch=B9").await;
    assert_eq!(data["updated"], json!(""), "{}", data["updated"]);
    let rates = data["rates"].as_array().expect("rates");
    assert!(
        rates.len() > 100,
        "every currency of the register, set or not: {}",
        rates.len()
    );
    assert!(
        rates.iter().all(|r| r["code"] != json!("THB")),
        "THB is not a rate"
    );
    for r in rates {
        for field in ["buy", "sell", "spread", "partnerCost", "partnerSpread"] {
            assert_eq!(r[field], Value::Null, "{}.{field}: {r}", r["code"]);
        }
        assert_eq!(r["updated"], json!(""), "{r}");
        assert_eq!(r["partnerName"], json!(""), "{r}");
        assert!(r["symbol"].is_string(), "symbol is a string: {r}");
        assert!(
            r["name"].as_str().is_some_and(|n| !n.is_empty()),
            "a name: {r}"
        );
    }
    room.finish().await;
}

/// The row's `updated` is exactly the text `adminSaveRate` takes back as `previousUpdated`, so
/// the screen's form can send it as it was read; any other text is refused as edited elsewhere.
#[actix_web::test]
async fn a_rows_updated_is_the_previous_updated_a_save_takes() {
    let room = Room::open().await;
    room.save_rate(MAIN, "USD", "30", "40", None).await;
    let data = room.rates("OWNER", "branch=MAIN").await;
    let updated = rate_of(&data, "USD")["updated"]
        .as_str()
        .expect("updated is a string")
        .to_owned();
    assert!(!updated.is_empty(), "a set rate has its time");

    let stale = room
        .save_rate_as("OWNER", MAIN, "USD", ("31", "41"), None, "")
        .await;
    stale.assert_refused(
        400,
        "เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน",
        "an empty previousUpdated for a set rate",
    );
    let saved = room
        .save_rate_as("OWNER", MAIN, "USD", ("31", "41"), None, &updated)
        .await;
    assert_eq!(saved.status, 200, "{:.300}", saved.text);
    let usd = room.rates("OWNER", "branch=MAIN").await;
    assert_eq!(rate_of(&usd, "USD")["buy"], json!("31.0000"), "{usd}");

    // a currency never set: previousUpdated is its "" updated
    let eur = rate_of(&usd, "EUR")["updated"]
        .as_str()
        .expect("updated")
        .to_owned();
    assert_eq!(eur, "");
    let saved = room
        .save_rate_as("OWNER", MAIN, "EUR", ("37", "38"), None, &eur)
        .await;
    assert_eq!(saved.status, 200, "{:.300}", saved.text);
    room.finish().await;
}

#[actix_web::test]
async fn the_owner_reads_the_branch_named_and_staff_their_own() {
    let room = Room::open().await;
    room.save_rate(B9, "USD", "30", "40", None).await;
    room.save_rate(MAIN, "USD", "31", "32", None).await;

    let nine = room.rates("OWNER", "branch=B9").await;
    assert_eq!(rate_of(&nine, "USD")["buy"], json!("30.0000"), "{nine}");
    for (who, query) in [
        ("OWNER", "branch=MAIN"),
        ("OWNER", ""),
        ("RATES", "branch=B9"),
        ("FRONT", "branch=B9"),
    ] {
        let data = room.rates(who, query).await;
        assert_eq!(
            rate_of(&data, "USD")["buy"],
            json!("31.0000"),
            "{who} {query:?}: MAIN's"
        );
        let page = room.history(who, query).await;
        assert_eq!(page.len(), 1, "{who} {query:?}: MAIN's one change");
        assert_eq!(page[0]["buy"], json!("31.0000"), "{who} {query:?}");
    }
    room.finish().await;
}

// ── ticket §8: saves, re-read ────────────────────────────────────────────────

/// A save as `RATES` or the owner is saved and the same `since` then answers it; every other
/// caller gets exactly the refusal `Policy` gives `(Rpc, adminSaveRate)` and the view stays put.
#[actix_web::test]
async fn a_save_is_saved_for_rates_and_the_owner_and_refused_with_the_policys_answer_otherwise() {
    let room = Room::open().await;
    let mut buy = 30;
    for (who, actor) in callers().into_iter().filter(|(w, _)| !w.is_empty()) {
        let before = room.view("OWNER", RATES, "branch=MAIN").await;
        let since = before["version"].as_i64().expect("a version");
        let previous = room.previous(MAIN, "USD").await;
        buy += 1;
        let answer = room
            .save_rate_as(
                &who,
                MAIN,
                "USD",
                (&buy.to_string(), &(buy + 1).to_string()),
                None,
                &previous,
            )
            .await;
        let policy = Policy::allow(
            actor.as_ref(),
            EntryPath::Rpc,
            &Action::new("adminSaveRate"),
        );
        let after = room
            .view("OWNER", RATES, &format!("branch=MAIN&since={since}"))
            .await;
        match policy {
            Ok(()) => {
                assert_eq!(answer.status, 200, "{who} saves: {:.300}", answer.text);
                assert_eq!(after["changed"], json!(true), "{who}: re-read {after}");
                assert_eq!(
                    rate_of(&after["data"], "USD")["buy"],
                    json!(format!("{buy}.0000")),
                    "{who}: the saved rate"
                );
            }
            Err(e) => {
                answer.assert_refused(e.status(), &e.to_string(), &who);
                assert_eq!(
                    after,
                    json!({ "version": since, "changed": false }),
                    "{who}: refused, nothing moved"
                );
                buy -= 1;
            }
        }
    }
    room.finish().await;
}

// ── §4.2: every writer moves the version ─────────────────────────────────────

/// One write per writer of what `admin_rates` reads, each through `BranchTx`: the same `since`
/// then answers `changed: true` with the write in it; before it, an equal `since` is exactly
/// unchanged. A write in another branch does not move this one.
#[actix_web::test]
async fn every_writer_of_the_rates_moves_the_version_it_answers() {
    let room = Room::open().await;
    let q = |since: i64| format!("branch=MAIN&since={since}");

    let v = room.version(MAIN).await;
    assert_eq!(
        room.view("OWNER", RATES, &q(v)).await,
        json!({ "version": v, "changed": false }),
        "an equal since: exactly the version and changed: false"
    );

    // a write in B9 is not MAIN's
    room.save_rate(B9, "USD", "30", "40", None).await;
    assert_eq!(
        room.view("OWNER", RATES, &q(v)).await,
        json!({ "version": v, "changed": false }),
        "B9's save does not move MAIN"
    );

    let mut since = v;
    for writer in ["adminSaveRate", "kiosk configure", "adminCreateBranch"] {
        let expect_buy = match writer {
            "adminSaveRate" => {
                room.save_rate(MAIN, "USD", "31", "32", None).await;
                Some(("USD", "31.0000"))
            }
            "kiosk configure" => {
                // #603 item 2: the euro is published first (moves no version)
                publication_seed::publish(
                    &room.desk.db.pool,
                    MAIN,
                    chrono::Utc::now(),
                    &[publication_seed::both("EUR", "37.5", "38.5")],
                )
                .await;
                let answer = room
                    .post(
                        "OWNER",
                        "/api/kiosk",
                        json!({ "action": "configure", "payload": {
                            "branchId": MAIN, "code": "EUR", "enabled": true,
                            "buy": "37.5", "sell": "38.5",
                            "previousUpdated": room.previous(MAIN, "EUR").await,
                            "partnerName": "", "partnerCost": null } }),
                    )
                    .await;
                assert_eq!(answer.status, 200, "configure: {:.300}", answer.text);
                // #603 item 1: the switch and its audit line move the version; no rate is saved
                None
            }
            "adminCreateBranch" => {
                let answer = room
                    .rpc(
                        "OWNER",
                        "adminCreateBranch",
                        json!({ "branchId": MAIN, "requestId": request_id(), "name": "Riverside" }),
                    )
                    .await;
                assert_eq!(
                    answer.status, 200,
                    "adminCreateBranch: {:.300}",
                    answer.text
                );
                None
            }
            _ => unreachable!(),
        };
        let now = room.version(MAIN).await;
        assert!(
            now > since,
            "{writer} moved MAIN's version ({since} → {now})"
        );
        let body = room.view("OWNER", RATES, &q(since)).await;
        assert_eq!(body["version"], json!(now), "after {writer}: {body}");
        assert_eq!(body["changed"], json!(true), "after {writer}: {body}");
        if let Some((code, buy)) = expect_buy {
            assert_eq!(
                rate_of(&body["data"], code)["buy"],
                json!(buy),
                "after {writer}"
            );
        }
        assert_eq!(
            room.view("OWNER", RATES, &q(now)).await,
            json!({ "version": now, "changed": false }),
            "after {writer}, the new version is unchanged"
        );
        since = now;
    }
    room.finish().await;
}

// ── admin_rate_history: the paged history ────────────────────────────────────

/// Newest first, the snapshot's `rateHistory` row for row (design §12.14); paged by `before` =
/// the last row's `id` until a short page, so the dialog walks it to the end (`walkPages`).
#[actix_web::test]
async fn the_history_is_the_snapshots_rate_history_newest_first_paged_by_id() {
    let room = Room::open().await;
    room.save_rate(MAIN, "USD", "30", "40", None).await;
    room.save_rate(MAIN, "EUR", "37", "38", Some(("Vasu", "37.5")))
        .await;
    room.save_rate(B9, "USD", "1", "2", None).await;
    room.save_rate(MAIN, "USD", "31", "41", None).await;
    let staff_previous = room.previous(MAIN, "GBP").await;
    let by_rates = room
        .save_rate_as("RATES", MAIN, "GBP", ("44", "45"), None, &staff_previous)
        .await;
    assert_eq!(by_rates.status, 200, "{:.300}", by_rates.text);

    let snapshot = room.snapshot(MAIN).await;
    let theirs = snapshot["rateHistory"].as_array().expect("rateHistory");
    assert_eq!(theirs.len(), 4, "MAIN's four changes: {snapshot}");

    let all = room.history("OWNER", "branch=MAIN").await;
    assert_eq!(all.len(), 4, "every change of MAIN, none of B9's");
    for (i, (ours, theirs)) in all.iter().zip(theirs).enumerate() {
        assert_eq!(keys(ours), set_of(&HISTORY_FIELDS), "row {i}: {ours}");
        assert!(
            ours["id"].as_str().is_some_and(|id| !id.is_empty()),
            "row {i}: id is a string: {ours}"
        );
        for field in ["code", "updated", "actor", "partnerName"] {
            assert_eq!(ours[field], theirs[field], "row {i}.{field}");
        }
        for field in ["buy", "sell", "partnerCost"] {
            if theirs[field].is_null() {
                assert_eq!(ours[field], Value::Null, "row {i}.{field}");
            } else {
                assert_eq!(
                    rate4(&ours[field], &format!("row {i}.{field}")),
                    number(&theirs[field], field),
                    "row {i}.{field}"
                );
            }
        }
    }
    assert_eq!(
        all.iter().map(|r| r["code"].clone()).collect::<Vec<_>>(),
        vec![json!("GBP"), json!("USD"), json!("EUR"), json!("USD")],
        "newest first"
    );
    assert_ne!(
        all[0]["actor"], all[1]["actor"],
        "who saved each: the RATES employee, then the owner"
    );
    assert_eq!(all[1]["buy"], json!("31.0000"));
    assert_eq!(all[2]["partnerCost"], json!("37.5000"));

    // paging: two, then the rest after the second's id, then nothing
    let first = room.history("OWNER", "branch=MAIN&limit=2").await;
    assert_eq!(first, all[..2].to_vec(), "the first page");
    let before = first[1]["id"].as_str().expect("an id").to_owned();
    let second = room
        .history("OWNER", &format!("branch=MAIN&limit=2&before={before}"))
        .await;
    assert_eq!(second, all[2..].to_vec(), "the rows after the second");
    let last = all[3]["id"].as_str().expect("an id").to_owned();
    let none = room
        .history("OWNER", &format!("branch=MAIN&limit=2&before={last}"))
        .await;
    assert!(
        none.is_empty(),
        "nothing older than the first change: {none:?}"
    );
    room.finish().await;
}

/// The history reads past the first hundred rows: a dialog walking pages of 100 with `before`
/// sees every change exactly once, newest first (v2 shows the whole `rateHistory`).
#[actix_web::test]
async fn walking_pages_of_a_hundred_reads_the_whole_history_once() {
    let room = Room::open().await;
    let t0 = room.t0;
    for i in 0..230_i64 {
        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
             VALUES ($1, 'USD', $2::numeric, $3::numeric, $4, 'seed', $5)",
        )
        .bind(MAIN)
        .bind(format!("{}.0001", 30 + i))
        .bind(format!("{}.0002", 30 + i))
        .bind(t0 - Duration::seconds(10_000 - i))
        .bind(request_id())
        .execute(&room.desk.db.pool)
        .await
        .expect("seed a rate row");
    }
    let mut seen = Vec::new();
    let mut before: Option<String> = None;
    loop {
        let query = match &before {
            None => "branch=MAIN&limit=100".to_owned(),
            Some(id) => format!("branch=MAIN&limit=100&before={id}"),
        };
        let page = room.history("OWNER", &query).await;
        seen.extend(page.iter().map(|r| r["buy"].clone()));
        if page.len() < 100 {
            break;
        }
        before = Some(page[99]["id"].as_str().expect("an id").to_owned());
    }
    let expected: Vec<Value> = (0..230_i64)
        .rev()
        .map(|i| json!(format!("{}.0001", 30 + i)))
        .collect();
    assert_eq!(seen, expected, "every row once, newest first");
    room.finish().await;
}

/// `rates-reference.js:8`: each change carries the copy of the BOT reference the server fetched
/// when it was saved — the row of its own currency, rates as four-place decimal strings — or
/// `null` when it has none.
#[actix_web::test]
async fn a_history_row_carries_the_bot_reference_stored_with_it() {
    let room = Room::open().await;
    room.desk.bank.answer_next(reference(
        "2026-10-06T04:59:00.000Z",
        vec![
            bot_row("USD", 1, "33.4218", "33.5008", "33.8278"),
            bot_row("GBP", 1, "44.0195", "44.1299", "44.909"),
            bot_row("JPY", 100, "0.2095", "0.2101", "0.2163"),
        ],
    ));
    room.save_rate(MAIN, "GBP", "44", "45", None).await;
    room.save_rate(MAIN, "JPY", "0.2", "0.22", None).await;
    room.desk.bank.answer_nothing_special();
    room.save_rate(MAIN, "USD", "33", "34", None).await;
    // a row stored with no reference at all
    sqlx::query(
        "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
         VALUES ($1, 'EUR', 37, 38, $2, 'seed', $3)",
    )
    .bind(MAIN)
    .bind(room.tick())
    .bind(request_id())
    .execute(&room.desk.db.pool)
    .await
    .expect("seed a rate row without a reference");

    let all = room.history("OWNER", "branch=MAIN").await;
    let row = |code: &str| -> &Value {
        all.iter()
            .find(|r| r["code"] == json!(code))
            .unwrap_or_else(|| panic!("{code} in the history"))
    };

    assert_eq!(
        row("EUR")["botReference"],
        Value::Null,
        "no reference stored"
    );

    let usd = &row("USD")["botReference"];
    assert_eq!(usd["fetchedAt"], json!("2026-10-06T05:00:00.000Z"), "{usd}");
    assert_eq!(
        usd["rows"],
        json!([]),
        "the bank had no USD row then: {usd}"
    );

    let gbp = &row("GBP")["botReference"];
    assert_eq!(gbp["fetchedAt"], json!("2026-10-06T04:59:00.000Z"), "{gbp}");
    let rows = gbp["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "the row of its own currency: {gbp}");
    assert_eq!(keys(&rows[0]), set_of(&BOT_ROW_FIELDS), "{gbp}");
    assert_eq!(
        rows[0],
        json!({ "code": "GBP", "date": "2026-10-06", "unit": 1,
                "buyingSight": "44.0195", "buyingTransfer": "44.1299", "selling": "44.9090" }),
        "four places, as v2's rateNumber prints them"
    );

    let jpy = &row("JPY")["botReference"]["rows"][0];
    assert_eq!(jpy["unit"], json!(100), "JPY per 100 (ticket §9): {jpy}");
    assert_eq!(jpy["buyingSight"], json!("0.2095"), "{jpy}");
    room.finish().await;
}

// ── the generated types ──────────────────────────────────────────────────────

fn exported(view: &str) -> BTreeMap<String, String> {
    let entry = VIEWS
        .iter()
        .find(|v| v.name == view)
        .unwrap_or_else(|| panic!("the view {view} is in the generated table"));
    let dir: PathBuf =
        std::env::temp_dir().join(format!("xc-468-{}-{}", std::process::id(), next()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    (entry.export_types)(&dir).unwrap_or_else(|e| panic!("export {view}: {e}"));
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

/// `field: type` of an object type in generated TypeScript, as written.
fn has_field(ts: &str, field: &str, types: &[&str]) {
    let found = types.iter().any(|ty| {
        ts.contains(&format!("{field}: {ty},")) || ts.contains(&format!("{field}: {ty} "))
    });
    assert!(found, "`{field}: one of {types:?}` in:\n{ts}");
}

#[test]
fn both_views_are_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    for view in [RATES, HISTORY] {
        assert!(names.contains(&view), "{view} in {names:?}");
    }
}

#[test]
fn the_generated_types_carry_rates_as_strings_into_each_views_own_file() {
    let rates = exported(RATES);
    let ts = rates
        .get("admin_rates.ts")
        .unwrap_or_else(|| panic!("admin_rates.ts in {:?}", rates.keys()));
    for field in ["buy", "sell", "spread", "partnerCost", "partnerSpread"] {
        has_field(ts, field, &["string | null", "Rate | null"]);
    }
    for field in ["code", "name", "symbol", "updated", "partnerName"] {
        has_field(ts, field, &["string"]);
    }
    assert!(!ts.contains("bigint"), "no bigint on the wire:\n{ts}");
    for name in rates.keys() {
        assert!(
            name == "admin_rates.ts" || name == "common.ts",
            "{RATES} writes only its own file and common.ts: {name}"
        );
    }

    let history = exported(HISTORY);
    let ts = history
        .get("admin_rate_history.ts")
        .unwrap_or_else(|| panic!("admin_rate_history.ts in {:?}", history.keys()));
    for field in ["id", "code", "updated", "actor", "partnerName"] {
        has_field(ts, field, &["string"]);
    }
    for field in ["buy", "sell"] {
        has_field(ts, field, &["string", "Rate"]);
    }
    has_field(ts, "partnerCost", &["string | null", "Rate | null"]);
    for field in ["buyingSight", "buyingTransfer", "selling"] {
        has_field(ts, field, &["string", "Rate"]);
    }
    has_field(ts, "unit", &["number"]);
    assert!(!ts.contains("bigint"), "no bigint on the wire:\n{ts}");
    for name in history.keys() {
        assert!(
            name == "admin_rate_history.ts" || name == "common.ts",
            "{HISTORY} writes only its own file and common.ts (no DTO shared with \
             rates_reference, design O-ra1): {name}"
        );
    }
}
