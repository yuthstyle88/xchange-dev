//! Ticket #1106 (EX-11d): the A01 low-stock and cash drill-down opens every currency;
//! admin_cash gains a currency filter + DRILLDOWN_KEYS (design
//! `docs/design/blueprint/executive-finance.md` §4.4; the lane controller's G1–G3
//! ruling on #1071 and its EX-11 rulings on #1104).
//!
//! Written before the implementation by the test writer gemini/studio-11 (controller claude/e92db6). Do not edit, delete or weaken.
//!
//! **What is pinned** (ticket §7–§9):
//! - `currency`: three upper-case ASCII letters. Anything else gives `400 invalidRequest`,
//!   field `currency`.
//! - A well-formed currency code the branch never traded (or does not hold) gives an empty
//!   stock list (`stock: []`), not an error; shift banner and THB cash summary (stats) stay
//!   unchanged.
//! - With `currency=USD` on a branch holding USD and JPY: exactly one row in stock (USD).
//! - With `currency=USD` and `all=true`: still exactly one row (USD).
//! - With none of the new keys: the answer equals the answer before this change.
//! - `since` -> `{changed:false}` works with `currency` filter as before.
//! - `DRILLDOWN_KEYS` in `dto/v3/admin_cash.rs` declares `all` and `currency` (G2),
//!   and `accepted_keys("admin_cash")` accepts `branch`, `all`, and `currency`.
//! - A01's drill-down to `admin_cash` (both `available_cash` metric and low-stock alert)
//!   passes `currency` in `filter` and no longer lists `currency` in `dropped`.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod shift_setup_common;
mod snapshot_common;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, MAIN, sign_in};
use serde_json::{Value, json};
use xchange::application::executive::drilldown::{Drilldown, accepted_keys};

const ORIGIN: &str = "http://desk.test";
const CASH: &str = "admin_cash";
const BUY_RATE: &str = "30";
const SELL_RATE: &str = "40";

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

/// The source text of `rel` under `api/`, whitespace removed.
fn flat_source(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} exists: {e}", path.display()))
        .split_whitespace()
        .collect()
}

/// `stock` as `code → (opening, net, expected)`, keeping the wire's strings.
fn stock_of(data: &Value) -> Vec<(String, String, String, String)> {
    data["stock"]
        .as_array()
        .unwrap_or_else(|| panic!("stock is an array in {data}"))
        .iter()
        .map(|row| {
            let s = |k: &str| {
                row[k]
                    .as_str()
                    .unwrap_or_else(|| panic!("stock.{k} is a string: {row}"))
                    .to_string()
            };
            (s("code"), s("opening"), s("net"), s("expected"))
        })
        .collect()
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = vec![compat_common::Person {
            key: "OWNER".to_string(),
            kind: "owner".to_string(),
            id: "owner".to_string(),
            username: "owner".to_string(),
            name: "Owner".to_string(),
            role: "OWNER".to_string(),
            branch_id: None,
            must_change: false,
        }];
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        let room = Room {
            desk,
            app,
            cookies,
            t0,
        };
        room.set_rate(MAIN, "USD", BUY_RATE, SELL_RATE).await;
        room
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        rates_re4e_publish::set_counter_rate(&self.desk.db.pool, branch, code, buy, sell).await;
    }

    async fn open_shift(&self, branch: &str) -> String {
        self.open_shift_with(branch, json!({ "THB": 100000, "USD": 1000 }))
            .await
    }

    async fn open_shift_with(&self, branch: &str, opening: Value) -> String {
        shift_setup_common::open_shift_by_count(&self.desk.db.pool, branch, opening, self.tick())
            .await
    }

    async fn get_view(&self, query: &str) -> Answer {
        let path = if query.is_empty() {
            format!("/api/v3/views/{CASH}")
        } else {
            format!("/api/v3/views/{CASH}?{query}")
        };
        let call = get(&path).cookie(&self.cookies["OWNER"]);
        self.app.send(call).await
    }

    async fn cash(&self, query: &str) -> Value {
        let answer = self.get_view(query).await;
        assert_eq!(answer.status, 200, "GET {query}: {}", answer.text);
        let body = answer.json();
        assert_eq!(body["changed"], json!(true), "GET {query}: {body}");
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("data in {body}"))
    }
}

// ------------------------------------------------------------ §9 edge cases & §8 input rules

/// §9 row 1: `currency=USD` on a branch holding USD and JPY: one row, USD.
#[actix_web::test]
async fn currency_usd_on_a_branch_holding_usd_and_jpy_returns_one_usd_row() {
    let room = Room::open().await;
    room.set_rate(MAIN, "JPY", "0.25", "0.30").await;
    room.open_shift(MAIN).await;

    let data = room.cash("branch=MAIN&currency=USD").await;
    let stock = stock_of(&data);
    assert_eq!(
        stock.len(),
        1,
        "currency=USD on a branch holding USD and JPY must return exactly one stock row: {stock:?}"
    );
    assert_eq!(stock[0].0, "USD");
    assert_eq!(stock[0].1, "1000.00");
    assert_eq!(stock[0].2, "0.00");
    assert_eq!(stock[0].3, "1000.00");
}

/// §9 row 2: `currency=USD` and `all=true`: still one row.
#[actix_web::test]
async fn currency_usd_and_all_true_still_returns_one_usd_row() {
    let room = Room::open().await;
    room.set_rate(MAIN, "JPY", "0.25", "0.30").await;
    room.open_shift(MAIN).await;

    let data = room.cash("branch=MAIN&currency=USD&all=true").await;
    let stock = stock_of(&data);
    assert_eq!(
        stock.len(),
        1,
        "currency=USD and all=true must still return only one row: {stock:?}"
    );
    assert_eq!(stock[0].0, "USD");
}

/// §9 row 3 & §8 row 1: `currency=EUR` with no EUR row: no row, the banner unchanged;
/// a well-formed code the branch never traded gives an empty answer, not an error.
#[actix_web::test]
async fn currency_eur_with_no_eur_row_returns_no_stock_row_and_banner_unchanged() {
    let room = Room::open().await;
    room.set_rate(MAIN, "JPY", "0.25", "0.30").await;
    let shift_id = room.open_shift(MAIN).await;

    let data = room.cash("branch=MAIN&currency=EUR").await;
    let stock = stock_of(&data);
    assert!(
        stock.is_empty(),
        "currency=EUR with no EUR row must return empty stock rows: {stock:?}"
    );

    // Shift banner and THB cash summary (stats) stay unchanged (§8)
    assert!(
        data["shift"].is_object(),
        "shift banner must be present: {data}"
    );
    assert_eq!(data["shift"]["id"], shift_id);
    let teller_id: String = sqlx::query_scalar(
        "SELECT counted_by::text FROM cash_counts WHERE shift_id = $1 AND kind = 'OPEN'",
    )
    .bind(&shift_id)
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("open count teller");
    assert_ne!(teller_id, "owner");
    assert_eq!(data["shift"]["staff"], teller_id);

    assert!(data["stats"].is_object(), "stats must be present: {data}");
    assert_eq!(data["stats"]["opening"], "100000.00");
}

/// §9 row 4 & §8 row 1: Bad `currency`: 400, field `currency`.
/// Three upper-case ASCII letters required. Anything else gives 400 invalidRequest, field currency.
#[actix_web::test]
async fn bad_currency_refuses_with_400_invalid_request_field_currency() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;

    for bad in ["usd", "US", "USDT", "U$D", "U1D", "123", "us_"] {
        let answer = room.get_view(&format!("branch=MAIN&currency={bad}")).await;
        assert_eq!(
            answer.status, 400,
            "bad currency '{bad}' must answer 400: {}",
            answer.text
        );
        let body = answer.json();
        assert_eq!(
            body["error"]["code"], "invalidRequest",
            "bad currency '{bad}' code must be invalidRequest: {body}"
        );
        assert_eq!(
            body["error"]["field"], "currency",
            "bad currency '{bad}' field must be currency: {body}"
        );
        assert!(
            body.get("data").is_none(),
            "bad currency '{bad}' must have no data in {body}"
        );
    }
}

/// §9 row 5: No new key: the answer equals the answer before this change.
#[actix_web::test]
async fn no_new_key_answers_identically_to_the_existing_view() {
    let room = Room::open().await;
    room.set_rate(MAIN, "JPY", "0.25", "0.30").await;
    room.open_shift(MAIN).await;

    let data_default = room.cash("branch=MAIN").await;
    let stock_default = stock_of(&data_default);
    let codes_default: Vec<String> = stock_default.iter().map(|r| r.0.clone()).collect();
    assert_eq!(codes_default, vec!["THB", "JPY", "USD"]);

    let data_all = room.cash("branch=MAIN&all=true").await;
    let stock_all = stock_of(&data_all);
    assert_eq!(stock_all.len(), 153);
    assert_eq!(stock_all[0].0, "THB");
}

/// §8 row 3: `since` -> `{changed:false}` works as before with currency filter.
#[actix_web::test]
async fn since_returns_changed_false_with_currency_filter() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;

    let call =
        get("/api/v3/views/admin_cash?branch=MAIN&currency=USD").cookie(&room.cookies["OWNER"]);
    let answer = room.app.send(call).await;
    assert_eq!(answer.status, 200);
    let body = answer.json();
    assert_eq!(body["changed"], json!(true));
    let version = body["version"].as_i64().expect("branch version as integer");

    let call_since = get(&format!(
        "/api/v3/views/admin_cash?branch=MAIN&currency=USD&since={version}"
    ))
    .cookie(&room.cookies["OWNER"]);
    let answer_since = room.app.send(call_since).await;
    assert_eq!(answer_since.status, 200);
    let body_since = answer_since.json();
    assert_eq!(
        body_since["changed"],
        json!(false),
        "same since must return changed:false: {body_since}"
    );
    assert!(body_since.get("data").is_none());
}

/// §7 (G2) & §11 step 2: `DRILLDOWN_KEYS` in `dto/v3/admin_cash.rs` is exactly `all, currency`,
/// and `accepted_keys("admin_cash")` in `drilldown.rs` accepts `branch`, `all`, and `currency`.
#[test]
fn drilldown_keys_in_dto_are_all_and_currency_and_accepted_by_drilldown_arm() {
    let dto = flat_source("src/presentation/dto/v3/admin_cash.rs");
    assert!(
        dto.contains("pubcurrency:Option<String>"),
        "AdminCashFilter must declare `pub currency: Option<String>`: {dto}"
    );
    let at = dto
        .find("pubconstDRILLDOWN_KEYS:&[&str]=&[")
        .expect("dto/v3/admin_cash.rs declares `pub const DRILLDOWN_KEYS: &[&str]`");
    let start = at + "pubconstDRILLDOWN_KEYS:&[&str]=&[".len();
    let end = start + dto[start..].find("]").expect("the list ends");
    let mut keys: Vec<&str> = dto[start..end]
        .split(',')
        .map(|k| k.trim_matches('"'))
        .filter(|k| !k.is_empty())
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["all", "currency"]);

    let accepted = accepted_keys("admin_cash");
    for key in ["branch", "all", "currency"] {
        assert!(
            accepted.contains(&key),
            "admin_cash accepts {key}: {accepted:?}"
        );
    }
}

/// §9 row 6 & §4.4: After this change, A01's available_cash drill-down to admin_cash passes
/// `currency` in `filter` and no longer lists `currency` in `dropped`.
#[actix_web::test]
async fn a01s_available_cash_drilldown_passes_currency_in_filter_and_not_in_dropped() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;

    let call = get("/api/executive/overview?from=2026-10-06&to=2026-10-06&currency=USD")
        .cookie(&room.cookies["OWNER"]);
    let answer = room.app.send(call).await;
    assert_eq!(answer.status, 200, "overview: {}", answer.text);
    let body = answer.json();
    let branches = body["data"]["branches"].as_array().expect("branches array");
    let main_row = branches
        .iter()
        .find(|b| b["branchId"] == MAIN)
        .expect("MAIN branch row");

    let dd = &main_row["availableCash"]["drilldown"];
    assert_eq!(dd["view"], "admin_cash", "{dd}");
    assert_eq!(dd["filter"]["branch"], MAIN, "{dd}");
    assert_eq!(
        dd["filter"]["currency"], "USD",
        "A01 available_cash drilldown must pass currency in filter: {dd}"
    );
    assert_eq!(
        dd["dropped"],
        json!([]),
        "A01 available_cash drilldown must drop nothing: {dd}"
    );
}

/// §9 row 6 & title: Low-stock alert drill-down to admin_cash passes `currency` in `filter`
/// and drops nothing.
#[test]
fn a01s_low_stock_alert_drilldown_passes_currency_in_filter_and_not_in_dropped() {
    let dd = Drilldown::to(
        "admin_cash",
        vec![
            ("branch", MAIN.to_string()),
            ("currency", "USD".to_string()),
        ],
    );
    assert_eq!(dd.view, "admin_cash");
    assert_eq!(dd.filter.get("branch").map(|s| s.as_str()), Some(MAIN));
    assert_eq!(
        dd.filter.get("currency").map(|s| s.as_str()),
        Some("USD"),
        "low-stock drilldown to admin_cash must accept currency in filter: {dd:?}"
    );
    assert_eq!(
        dd.dropped,
        Vec::<&str>::new(),
        "low-stock drilldown to admin_cash must drop nothing: {dd:?}"
    );
}
