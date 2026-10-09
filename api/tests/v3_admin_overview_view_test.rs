//! Ticket #461 (V2-overview, `docs/design/xchange-v3-web.md` §5.1, §12.1, §12.2, §12.6): the
//! bounded view the admin overview reads. Written before the implementation by the lane
//! controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/admin_overview?branch=&since=&from=&to=` — a `BranchView` inheriting
//!   `(EntryPath::Rpc, "adminSnapshot")`, filtered by the period (`from` / `to`, design §12.1:
//!   ISO days, both inclusive, `''` or absent = open on that side; `from > to` →
//!   `400 periodOrder`, `field: "to"`; a malformed day → `400 invalidRequest` naming it).
//! - `data` = `{period, totals, latest, status}`:
//!   - `period`: `{from, to}` as answered (`''` where open);
//!   - `totals`: v2's `totals` (`admin-html:60`) and the figures the overview prints from it
//!     (`:64-66`), computed by the server over the period's LIVE exchanges of the branch. Money is
//!     a decimal string with two places (`"0.00"` when there is none, ticket §9); `net` is signed
//!     (`received − paid`, `:66`); `buyShare` / `sellShare` are the BUY and SELL bars' widths
//!     (`:65`, `n / volume × 100`, `"0"`-valued when the volume is zero) as decimal strings, so the
//!     screen never divides (W7); the counts are JSON numbers. v3 stores an exchange only once it
//!     is saved, so every stored exchange is COMPLETED (`snapshot.rs:15-16`) and
//!     `cancelledCount` / `pendingCount` (the fourth card's note, `:64`) are 0 — see the comment on
//!     the ticket;
//!   - `latest`: up to five of the period's exchanges, newest first by the time they were recorded
//!     (`:67` sorts by `created`), each a `TransactionRow` (V2-parts #476);
//!   - `status` (not period-bound, `admin-workspace.js:152-155`): `{shiftOpen, pendingCount}`.
//! - LIVE only (design §12.5): a DEMO exchange is in nothing the view answers, as the snapshot's
//!   `transactions` lists only LIVE ones (`infrastructure/compat/snapshot_sources.rs:101`).
//! - §4.2: every writer of what the view reads goes through `BranchTx` — the exchange save, shift
//!   open, stock move, shift close and abandon (ticket §7) — one write each, then the same `since`
//!   answers `changed: true`.
//!
//! Field names beyond the ticket's [proposed] list (`buyCount`, `sellCount`, `buyShare`,
//! `sellShare`, `cancelledCount`, `pendingCount` inside `totals`) are the figures `:64-65` print
//! and are fixed here.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`); writes are real `/api/rpc` actions and
//! exchanges are saved through the real `SaveTransactionWorkflow` behind the routes, as
//! `v3_admin_parts_views_test.rs` does. The test's clock is advanced before every write, so no two
//! exchanges share a record time.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`) for the view tests; nothing skips.

mod common;
mod compat_common;
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
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";
const OVERVIEW: &str = "admin_overview";

const MAIN: &str = "MAIN";
const B9: &str = "B9";

/// The counter rates of USD in every branch of the room: the shop buys at 30, sells at 40.
const BUY_RATE: &str = "30";
const SELL_RATE: &str = "40";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

/// `TransactionRow`'s fields (V2-parts #476, design §12.2).
const ROW_FIELDS: [&str; 25] = [
    "id",
    "date",
    "created",
    "type",
    "currency",
    "amount",
    "rate",
    "principal",
    "fee",
    "bankFee",
    "settlement",
    "method",
    "customer",
    "customerId",
    "staff",
    "status",
    "notes",
    "cancelReason",
    "shiftId",
    "referenceRate",
    "referenceValue",
    "referenceSpread",
    "referencePercent",
    "paymentRef",
    "slipUrl",
];

const DATA_FIELDS: [&str; 4] = ["period", "totals", "latest", "status"];
const TOTALS_MONEY: [&str; 7] = [
    "volume",
    "buyPrincipal",
    "sellPrincipal",
    "received",
    "paid",
    "net",
    "fees",
];
const TOTALS_SHARES: [&str; 2] = ["buyShare", "sellShare"];
const TOTALS_COUNTS: [&str; 5] = [
    "buyCount",
    "sellCount",
    "completedCount",
    "cancelledCount",
    "pendingCount",
];
const STATUS_FIELDS: [&str; 3] = ["shiftOpen", "pendingCount", "openedBy"];

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

/// A decimal string on the wire, read exactly (never through a float).
fn decimal(value: &Value, what: &str) -> Decimal {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{what}: a decimal string on the wire (W7), got {value}"));
    Decimal::from_str(text)
        .unwrap_or_else(|e| panic!("{what}: a decimal string, got {text:?}: {e}"))
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal literal")
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

fn view_path(query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{OVERVIEW}")
    } else {
        format!("/api/v3/views/{OVERVIEW}?{query}")
    }
}

/// One exchange to save.
#[derive(Clone, Copy)]
struct Ex {
    side: &'static str,
    amount: &'static str,
    fee: &'static str,
    bank_fee: &'static str,
    value_date: &'static str,
    live: bool,
}

impl Ex {
    fn buy(amount: &'static str, fee: &'static str, value_date: &'static str) -> Ex {
        Ex {
            side: "BUY",
            amount,
            fee,
            bank_fee: "0",
            value_date,
            live: true,
        }
    }

    fn sell(amount: &'static str, fee: &'static str, value_date: &'static str) -> Ex {
        Ex {
            side: "SELL",
            amount,
            fee,
            bank_fee: "0",
            value_date,
            live: true,
        }
    }

    fn bank_fee(self, bank_fee: &'static str) -> Ex {
        Ex { bank_fee, ..self }
    }

    fn demo(self) -> Ex {
        Ex {
            live: false,
            ..self
        }
    }
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl Room {
    /// `MAIN` (from the migration) and `B9`; the owner and one employee of every staff role in
    /// `MAIN`, all signed in; the USD counter rate 30 / 40 in both branches.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, "Nine").await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        let room = Room {
            desk,
            app,
            cookies,
            t0,
        };
        for branch in [MAIN, B9] {
            room.set_rate(branch, "USD", BUY_RATE, SELL_RATE).await;
        }
        room
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

    /// The view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, who: &str, query: &str) -> Value {
        let path = view_path(query);
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
    async fn data(&self, who: &str, query: &str) -> Value {
        let body = self.view(who, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {}: {body}",
            view_path(query)
        );
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: data in {body}", view_path(query)))
    }

    async fn post(&self, who: &str, action: &str, payload: Value) -> Answer {
        self.tick();
        let call = Call::action(ORIGIN, RPC, action, payload).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    async fn ok(&self, who: &str, action: &str, payload: Value) -> Value {
        let answer = self.post(who, action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "{action} as {who} with {payload}: {:.400}",
            answer.text
        );
        answer.json()
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
             VALUES ($1, $2, $3::numeric, $4::numeric, now(), 'test', $5)",
        )
        .bind(branch)
        .bind(code)
        .bind(buy)
        .bind(sell)
        .bind(request_id())
        .execute(&self.desk.db.pool)
        .await
        .expect("insert a counter rate");
    }

    /// The owner opens a shift in `branch` through `/api/rpc`.
    async fn open_shift(&self, branch: &str) {
        self.ok(
            "OWNER",
            "adminOpenShift",
            json!({ "branchId": branch, "requestId": request_id(),
                    "opening": { "THB": 100000, "USD": 1000 } }),
        )
        .await;
    }

    async fn open_shift_id(&self, branch: &str) -> String {
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch)
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("an open shift")
    }

    async fn expected_of_open_shift(&self, branch: &str) -> Value {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT b.code::text, b.expected::text FROM shift_balances b \
             JOIN shifts s ON s.id = b.shift_id WHERE s.branch_id = $1 AND s.status = 'OPEN'",
        )
        .bind(branch)
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read the open shift's balances");
        Value::Object(
            rows.into_iter()
                .map(|(c, e)| (c.trim().to_owned(), Value::String(e)))
                .collect(),
        )
    }

    /// The owner closes the open shift of `branch`, counting what is expected.
    async fn close_shift(&self, branch: &str) {
        let shift = self.open_shift_id(branch).await;
        let expected = self.expected_of_open_shift(branch).await;
        self.ok(
            "OWNER",
            "adminCloseShift",
            json!({ "branchId": branch, "requestId": request_id(), "shiftId": shift,
                    "expected": expected, "counted": expected, "note": "นับเงินปิดกะ" }),
        )
        .await;
    }

    /// The owner moves stock into the open shift of `branch`.
    async fn move_stock(&self, branch: &str) {
        let shift = self.open_shift_id(branch).await;
        self.ok(
            "OWNER",
            "adminMoveStock",
            json!({ "branchId": branch, "requestId": request_id(), "shiftId": shift,
                    "currency": "USD", "direction": "IN", "amount": 10, "reason": "from the safe" }),
        )
        .await;
    }

    /// The owner gives up a request id nothing applied, which `adminAbandonRequest` writes down.
    async fn abandon(&self, branch: &str) {
        let answer = self
            .ok(
                "OWNER",
                "adminAbandonRequest",
                json!({ "branchId": branch, "requestId": request_id() }),
            )
            .await;
        assert_eq!(
            answer["requestResolution"],
            json!("CANCELLED"),
            "a fresh id is given up: {answer}"
        );
    }

    /// One exchange of USD saved through the real workflow behind the routes, at the counter rate
    /// of its side; a LIVE one needs an open shift. Its document ref.
    async fn save(&self, branch: &str, ex: Ex) -> String {
        let t = self.tick();
        let here = BranchId::new(branch);
        let mode = if ex.live { "LIVE" } else { "DEMO" };
        let person = ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: mode.to_owned(),
        };
        let customer = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&here, person.clone(), t)
            .await
            .expect("reserve the customer")
            .customer_id;
        let passport = self
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let rate = if ex.side == "BUY" {
            BUY_RATE
        } else {
            SELL_RATE
        };
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer,
            side: ex.side.to_owned(),
            currency: "USD".to_owned(),
            amount: ex.amount.to_owned(),
            rate: rate.to_owned(),
            fee: ex.fee.to_owned(),
            bank_fee: ex.bank_fee.to_owned(),
            method: "Cash".to_owned(),
            payment_ref: String::new(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: ex.value_date.to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip: None,
            reference_rate: None,
        };
        self.desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange in {branch}: {e:?}"))
            .document_ref
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

/// The figures of `totals` the tests compare, as `(field, expected)`; money exactly as the wire
/// carries it (two places), shares by decimal value, counts as numbers.
fn assert_totals(
    totals: &Value,
    money: &[(&str, &str)],
    shares: &[(&str, &str)],
    counts: &[(&str, u64)],
    what: &str,
) {
    for (field, expected) in money {
        assert_eq!(
            totals[*field],
            json!(expected),
            "{what}: totals.{field} as a two-place decimal string: {totals}"
        );
    }
    for (field, expected) in shares {
        assert_eq!(
            decimal(&totals[*field], &format!("{what}: totals.{field}")),
            dec(expected),
            "{what}: totals.{field}: {totals}"
        );
    }
    for (field, expected) in counts {
        assert_eq!(
            totals[*field],
            json!(expected),
            "{what}: totals.{field} as a JSON number: {totals}"
        );
    }
}

fn ids(latest: &Value) -> Vec<String> {
    latest
        .as_array()
        .unwrap_or_else(|| panic!("latest is a list: {latest}"))
        .iter()
        .map(|row| {
            row["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row's id is a string: {row}"))
                .to_owned()
        })
        .collect()
}

// ── who may read it ──────────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_overview_answers_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        let policy = Policy::allow(
            actor.as_ref(),
            EntryPath::Rpc,
            &Action::new("adminSnapshot"),
        );
        let answer = room.get(&who, &view_path("")).await;
        let what = format!("{OVERVIEW} as {who:?}");
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
        "the owner, the nine roles, the owner beside each"
    );
    assert_eq!(refused, 1, "nobody signed in");
    room.finish().await;
}

// ── the answer's shape and an empty period ───────────────────────────────────

#[actix_web::test]
async fn an_empty_period_answers_zero_totals_no_rows_and_the_period_asked() {
    let room = Room::open().await;
    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-01&to=2026-10-06")
        .await;
    assert_eq!(keys(&data), set_of(&DATA_FIELDS), "{data}");
    assert_eq!(
        data["period"],
        json!({ "from": "2026-10-01", "to": "2026-10-06" }),
        "the period answered"
    );
    let mut all = Vec::new();
    all.extend(TOTALS_MONEY);
    all.extend(TOTALS_SHARES);
    all.extend(TOTALS_COUNTS);
    assert_eq!(keys(&data["totals"]), set_of(&all), "{data}");
    assert_totals(
        &data["totals"],
        &TOTALS_MONEY.map(|f| (f, "0.00")),
        &TOTALS_SHARES.map(|f| (f, "0")),
        &TOTALS_COUNTS.map(|f| (f, 0)),
        "no exchange",
    );
    assert_eq!(data["latest"], json!([]), "no rows: {data}");
    assert_eq!(keys(&data["status"]), set_of(&STATUS_FIELDS), "{data}");
    assert_eq!(
        data["status"],
        json!({ "shiftOpen": false, "pendingCount": 0, "openedBy": null }),
        "{data}"
    );

    // no period named: open on both sides, answered as ''
    let data = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(data["period"], json!({ "from": "", "to": "" }), "{data}");
    room.finish().await;
}

// ── the totals ───────────────────────────────────────────────────────────────

/// v2's `totals` (`admin-html:60`) and what `:64-66` print from it, over the period's LIVE
/// exchanges of the branch: the volume and each side's principal, the THB received (SELL, principal
/// + fee) and paid (BUY, principal − fee), the customer fees (never the bank fee), the signed net,
/// the counts and each side's share of the volume.
#[actix_web::test]
async fn the_totals_are_v2s_sums_over_the_periods_live_exchanges_of_the_branch() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;
    room.open_shift(B9).await;
    // in MAIN, inside 2026-10-05 … 2026-10-06
    room.save(MAIN, Ex::buy("100", "50", "2026-10-05")).await; // 3000.00, paid 2950.00
    room.save(MAIN, Ex::buy("10", "0", "2026-10-06")).await; // 300.00, paid 300.00
    room.save(MAIN, Ex::sell("27.5", "20", "2026-10-06").bank_fee("15"))
        .await; // 1100.00, received 1120.00
    // none of these may count
    room.save(MAIN, Ex::buy("1", "0", "2026-10-04")).await; // before the period
    room.save(MAIN, Ex::sell("1", "0", "2026-10-07")).await; // after the period
    room.save(MAIN, Ex::buy("500", "10", "2026-10-06").demo())
        .await; // DEMO
    room.save(B9, Ex::sell("5", "1", "2026-10-06")).await; // another branch

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    assert_totals(
        &data["totals"],
        &[
            ("volume", "4400.00"),
            ("buyPrincipal", "3300.00"),
            ("sellPrincipal", "1100.00"),
            ("received", "1120.00"),
            ("paid", "3250.00"),
            ("net", "-2130.00"),
            ("fees", "70.00"),
        ],
        &[("buyShare", "75"), ("sellShare", "25")],
        &[
            ("buyCount", 2),
            ("sellCount", 1),
            ("completedCount", 3),
            ("cancelledCount", 0),
            ("pendingCount", 0),
        ],
        "MAIN, 2026-10-05 … 2026-10-06",
    );
    assert_eq!(ids(&data["latest"]).len(), 3, "the period's three: {data}");
    assert!(
        !data.to_string().contains("TEST-XC-"),
        "a DEMO exchange is in nothing the view answers: {data}"
    );

    // the same figures through the shop's net being positive
    room.save(MAIN, Ex::sell("100", "40", "2026-10-06")).await; // 4000.00, received 4040.00
    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    assert_totals(
        &data["totals"],
        &[
            ("volume", "8400.00"),
            ("buyPrincipal", "3300.00"),
            ("sellPrincipal", "5100.00"),
            ("received", "5160.00"),
            ("paid", "3250.00"),
            ("net", "1910.00"),
            ("fees", "110.00"),
        ],
        &[],
        &[("buyCount", 2), ("sellCount", 2), ("completedCount", 4)],
        "after one more SELL",
    );
    // 3300 / 8400 and 5100 / 8400 do not end: the shares are the server's decimals of them
    let buy = decimal(&data["totals"]["buyShare"], "buyShare");
    let sell = decimal(&data["totals"]["sellShare"], "sellShare");
    let exact_buy = dec("3300") * dec("100") / dec("8400");
    let exact_sell = dec("5100") * dec("100") / dec("8400");
    assert!(
        (buy - exact_buy).abs() < dec("0.01") && (sell - exact_sell).abs() < dec("0.01"),
        "buyShare {buy} ≈ {exact_buy}, sellShare {sell} ≈ {exact_sell} (within 0.01)"
    );
    room.finish().await;
}

/// Design §12.1: both ends inclusive, `''` or absent = open on that side, by the exchange's date.
#[actix_web::test]
async fn the_period_is_inclusive_at_both_ends_and_open_where_empty() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;
    let first = room.save(MAIN, Ex::buy("1", "0", "2026-10-01")).await;
    let third = room.save(MAIN, Ex::buy("3", "0", "2026-10-03")).await;
    let fifth = room.save(MAIN, Ex::buy("5", "0", "2026-10-05")).await;

    for (query, expected, volume) in [
        ("from=2026-10-03&to=2026-10-03", vec![&third], "90.00"),
        ("from=2026-10-03", vec![&fifth, &third], "240.00"),
        ("from=2026-10-03&to=", vec![&fifth, &third], "240.00"),
        ("to=2026-10-03", vec![&third, &first], "120.00"),
        ("from=&to=2026-10-03", vec![&third, &first], "120.00"),
        ("from=2026-10-02&to=2026-10-04", vec![&third], "90.00"),
        (
            "from=2026-10-01&to=2026-10-05",
            vec![&fifth, &third, &first],
            "270.00",
        ),
        ("", vec![&fifth, &third, &first], "270.00"),
        ("from=2026-10-06&to=2026-10-09", vec![], "0.00"),
    ] {
        let q = if query.is_empty() {
            "branch=MAIN".to_string()
        } else {
            format!("branch=MAIN&{query}")
        };
        let data = room.data("OWNER", &q).await;
        let expected: Vec<String> = expected.into_iter().cloned().collect();
        assert_eq!(ids(&data["latest"]), expected, "{query:?}: {data}");
        assert_eq!(data["totals"]["volume"], json!(volume), "{query:?}: {data}");
        assert_eq!(
            data["totals"]["completedCount"],
            json!(expected.len()),
            "{query:?}: {data}"
        );
    }
    room.finish().await;
}

/// Design §12.1: a bad filter is refused before the `since` shortcut, naming its field.
#[actix_web::test]
async fn a_period_out_of_order_or_malformed_is_refused_naming_its_field() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    for query in [
        "branch=MAIN&from=2026-10-07&to=2026-10-06".to_string(),
        format!("branch=MAIN&from=2026-10-07&to=2026-10-06&since={v}"),
    ] {
        let answer = room.get("OWNER", &view_path(&query)).await;
        assert_eq!(answer.status, 400, "{query}: {:.300}", answer.text);
        let body = answer.json();
        assert_eq!(
            body["error"],
            json!({ "code": "periodOrder", "field": "to",
                    "message": "วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด" }),
            "{query}: v2's date-error sentence (admin-html:57)"
        );
        assert!(body.get("data").is_none(), "{query}: no data");
    }
    for (query, field) in [
        ("branch=MAIN&from=2026-13-01", "from"),
        ("branch=MAIN&from=06/10/2026", "from"),
        ("branch=MAIN&to=2026-02-30", "to"),
        ("branch=MAIN&from=2026-10-01&to=yesterday", "to"),
    ] {
        let answer = room.get("OWNER", &view_path(query)).await;
        assert_eq!(answer.status, 400, "{query}: {:.300}", answer.text);
        let body = answer.json();
        assert_eq!(
            body["error"]["code"],
            json!("invalidRequest"),
            "{query}: {body}"
        );
        assert_eq!(body["error"]["field"], json!(field), "{query}: {body}");
    }
    room.finish().await;
}

// ── the latest five ──────────────────────────────────────────────────────────

/// `admin-html:67`: the period's rows sorted newest first by `created`, cut at five — by the time
/// each was recorded, not by its date and not by amount; each a `TransactionRow`.
#[actix_web::test]
async fn latest_is_the_five_most_recently_recorded_of_the_period_as_transaction_rows() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;
    // recorded in this order; the dates run the other way, so ordering by date would differ
    let dates = [
        "2026-10-06",
        "2026-10-06",
        "2026-10-05",
        "2026-10-05",
        "2026-10-04",
        "2026-10-04",
        "2026-10-03",
    ];
    let mut saved = Vec::new();
    for (i, date) in dates.iter().enumerate() {
        let ex = if i % 2 == 0 {
            Ex::buy("10", "1", date)
        } else {
            Ex::sell("10", "2", date)
        };
        saved.push(room.save(MAIN, ex).await);
    }
    // the newest of all, outside the period
    let outside = room.save(MAIN, Ex::buy("10", "0", "2026-10-01")).await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-03&to=2026-10-06")
        .await;
    let expected: Vec<String> = saved.iter().rev().take(5).cloned().collect();
    assert_eq!(
        ids(&data["latest"]),
        expected,
        "newest recorded first: {data}"
    );
    assert!(!ids(&data["latest"]).contains(&outside), "{data}");
    assert_eq!(
        data["totals"]["completedCount"],
        json!(7),
        "the totals are the whole period's, not the five rows': {data}"
    );

    let shift = room.open_shift_id(MAIN).await;
    for row in data["latest"].as_array().expect("a list") {
        assert_eq!(keys(row), set_of(&ROW_FIELDS), "a TransactionRow: {row}");
        let id = row["id"].as_str().expect("an id");
        let i = saved
            .iter()
            .position(|s| s == id)
            .expect("one of the saved");
        let buy = i % 2 == 0;
        assert_eq!(
            row["type"],
            json!(if buy { "BUY" } else { "SELL" }),
            "{row}"
        );
        assert_eq!(row["date"], json!(dates[i]), "{row}");
        assert_eq!(row["currency"], json!("USD"), "{row}");
        assert_eq!(row["status"], json!("COMPLETED"), "{row}");
        assert_eq!(row["customer"], json!("Somchai Jaidee"), "{row}");
        assert_eq!(row["shiftId"], json!(shift), "{row}");
        assert_eq!(decimal(&row["amount"], "amount"), dec("10"), "{row}");
        let (rate, principal, settlement) = if buy {
            ("30", "300", "299")
        } else {
            ("40", "400", "402")
        };
        assert_eq!(decimal(&row["rate"], "rate"), dec(rate), "{row}");
        assert_eq!(
            decimal(&row["principal"], "principal"),
            dec(principal),
            "{row}"
        );
        assert_eq!(
            decimal(&row["settlement"], "settlement"),
            dec(settlement),
            "v2's settlement (admin-html:55): {row}"
        );
    }
    room.finish().await;
}

// ── the status strip ─────────────────────────────────────────────────────────

/// `admin-workspace.js:152-155`: the branch's state now, whatever the period.
#[actix_web::test]
async fn status_is_the_branchs_shift_now_whatever_the_period() {
    let room = Room::open().await;
    let far = "branch=MAIN&from=2020-01-01&to=2020-01-01";
    assert_eq!(
        room.data("OWNER", far).await["status"],
        json!({ "shiftOpen": false, "pendingCount": 0, "openedBy": null })
    );
    room.open_shift(MAIN).await;
    room.save(MAIN, Ex::buy("10", "0", "2026-10-06")).await;
    let data = room.data("OWNER", far).await;
    assert_eq!(
        data["status"],
        json!({ "shiftOpen": true, "pendingCount": 0, "openedBy": "owner" }),
        "a period with nothing in it still shows the open shift: {data}"
    );
    assert_eq!(data["latest"], json!([]), "{data}");
    room.close_shift(MAIN).await;
    assert_eq!(
        room.data("OWNER", far).await["status"],
        json!({ "shiftOpen": false, "pendingCount": 0, "openedBy": null })
    );
    room.finish().await;
}

// ── branches ─────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_owner_reads_the_branch_named_and_staff_their_own() {
    let room = Room::open().await;
    room.open_shift(B9).await;
    let nine = room.save(B9, Ex::sell("10", "0", "2026-10-06")).await;

    let data = room.data("OWNER", "branch=B9").await;
    assert_eq!(ids(&data["latest"]), vec![nine.clone()], "{data}");
    assert_eq!(data["totals"]["volume"], json!("400.00"), "{data}");
    assert_eq!(data["status"]["shiftOpen"], json!(true), "{data}");

    for (who, query) in [
        ("OWNER", "branch=MAIN"),
        ("OWNER", ""),
        ("FRONT", "branch=B9"),
    ] {
        let data = room.data(who, query).await;
        assert_eq!(data["latest"], json!([]), "{who} {query:?}: MAIN's: {data}");
        assert_eq!(
            data["totals"]["volume"],
            json!("0.00"),
            "{who} {query:?}: {data}"
        );
        assert_eq!(
            data["status"]["shiftOpen"],
            json!(false),
            "{who} {query:?}: {data}"
        );
        assert!(
            !data.to_string().contains(&nine),
            "{who} {query:?}: B9's exchange"
        );
    }
    room.finish().await;
}

// ── §4.2: every writer moves the version ─────────────────────────────────────

/// One write per writer of what the view reads, each through `BranchTx`: the same `since` then
/// answers `changed: true` with the write in it; before it, an equal `since` is exactly unchanged.
#[actix_web::test]
async fn every_writer_of_the_overview_moves_the_version_it_answers() {
    let room = Room::open().await;
    let q = |since: i64| format!("branch=MAIN&from=2026-10-01&to=2026-10-31&since={since}");

    let v = room.version(MAIN).await;
    assert_eq!(
        room.view("OWNER", &q(v)).await,
        json!({ "version": v, "changed": false }),
        "an equal since: exactly the version and changed: false"
    );

    let mut since = v;
    let mut completed = 0;
    for writer in [
        "adminOpenShift",
        "save",
        "adminMoveStock",
        "adminAbandonRequest",
        "adminCloseShift",
    ] {
        match writer {
            "adminOpenShift" => room.open_shift(MAIN).await,
            "save" => {
                room.save(MAIN, Ex::buy("10", "0", "2026-10-06")).await;
                completed += 1;
            }
            "adminMoveStock" => room.move_stock(MAIN).await,
            "adminAbandonRequest" => room.abandon(MAIN).await,
            "adminCloseShift" => room.close_shift(MAIN).await,
            _ => unreachable!(),
        }
        let now = room.version(MAIN).await;
        assert!(
            now > since,
            "{writer} moved MAIN's version ({since} → {now})"
        );
        let body = room.view("OWNER", &q(since)).await;
        assert_eq!(body["version"], json!(now), "after {writer}: {body}");
        assert_eq!(body["changed"], json!(true), "after {writer}: {body}");
        assert_eq!(
            body["data"]["totals"]["completedCount"],
            json!(completed),
            "after {writer}: {body}"
        );
        assert_eq!(
            body["data"]["status"]["shiftOpen"],
            json!(writer != "adminCloseShift"),
            "after {writer}: {body}"
        );
        assert_eq!(
            room.view("OWNER", &q(now)).await,
            json!({ "version": now, "changed": false }),
            "after {writer}, the new version is unchanged"
        );
        since = now;
    }
    room.finish().await;
}

// ── the generated types ──────────────────────────────────────────────────────

fn exported() -> BTreeMap<String, String> {
    let entry = VIEWS
        .iter()
        .find(|v| v.name == OVERVIEW)
        .unwrap_or_else(|| panic!("the view {OVERVIEW} is in the generated table"));
    let dir: PathBuf =
        std::env::temp_dir().join(format!("xc-461-{}-{}", std::process::id(), next()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    (entry.export_types)(&dir).unwrap_or_else(|e| panic!("export {OVERVIEW}: {e}"));
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
fn admin_overview_is_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert!(names.contains(&OVERVIEW), "{OVERVIEW} in {names:?}");
}

#[test]
fn admin_overviews_types_carry_the_fields_money_as_strings_counts_as_numbers() {
    let files = exported();
    let ts = files
        .get("admin_overview.ts")
        .unwrap_or_else(|| panic!("admin_overview.ts in {:?}", files.keys()));
    for field in TOTALS_MONEY.iter().chain(TOTALS_SHARES.iter()) {
        has_field(ts, field, &["string", "Money", "Rate"]);
    }
    for field in TOTALS_COUNTS {
        has_field(ts, field, &["number"]);
    }
    has_field(ts, "shiftOpen", &["boolean"]);
    // the period answered: V0-C's shared `Period`, or its two strings
    if !ts.contains("period: Period,") {
        has_field(ts, "from", &["string"]);
        has_field(ts, "to", &["string"]);
    }
    has_field(ts, "latest", &["Array<TransactionRow>", "TransactionRow[]"]);
    assert!(!ts.contains("bigint"), "no bigint on the wire:\n{ts}");
    assert!(
        ts.contains("from \"./transaction_row\""),
        "latest is V2-parts' TransactionRow, not a copy:\n{ts}"
    );
}

/// Decision 2 (issuecomment-6067199516): status.openedBy carries the shift opener's name
/// when a shift is open, and is absent/null when closed.
#[actix_web::test]
async fn status_opened_by_shows_opener_when_open() {
    let room = Room::open().await;
    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-01&to=2026-10-06")
        .await;
    assert_eq!(data["status"]["openedBy"], Value::Null);
    assert_eq!(data["status"]["shiftOpen"], json!(false));

    room.open_shift(MAIN).await;
    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-01&to=2026-10-06")
        .await;
    assert_eq!(data["status"]["shiftOpen"], json!(true));
    assert_eq!(data["status"]["openedBy"], json!("owner"));

    room.close_shift(MAIN).await;
    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-01&to=2026-10-06")
        .await;
    assert_eq!(data["status"]["shiftOpen"], json!(false));
    assert_eq!(data["status"]["openedBy"], Value::Null);

    room.finish().await;
}

/// Midpoint rounding test (ruling item 4): proves half-away-from-zero is used
/// for principals and totals, and goes red under half-even (Banker's rounding) and truncation.
#[actix_web::test]
async fn totals_round_cents_at_midpoint_pins_half_away_from_zero() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;
    room.set_rate(MAIN, "USD", "30.025", "40.045").await;

    // BUY 1 USD @ 30.025: raw principal = 30.025.
    // Preceding hundredths digit is 2 (even).
    // - round_cents (half toward +inf): rounds to 30.03
    // - MidpointNearestEven (half-even): rounds to 30.02 (fails!)
    // - Truncation (round towards zero): truncates to 30.02 (fails!)
    save_midpoint_tx(&room, MAIN, "BUY", "1", "30.025", "2026-10-06").await;

    // SELL 1 USD @ 40.045: raw principal = 40.045.
    // Preceding hundredths digit is 4 (even).
    // - round_cents (half toward +inf): rounds to 40.05
    // - MidpointNearestEven: rounds to 40.04 (fails!)
    // - Truncation: truncates to 40.04 (fails!)
    save_midpoint_tx(&room, MAIN, "SELL", "1", "40.045", "2026-10-06").await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-06&to=2026-10-06")
        .await;

    // Total volume: 30.03 + 40.05 = 70.08
    // Under half-even: 30.02 + 40.04 = 70.06 (red)
    // Under truncation: 30.02 + 40.04 = 70.06 (red)
    assert_eq!(data["totals"]["volume"], json!("70.08"), "{data}");
    assert_eq!(data["totals"]["buyPrincipal"], json!("30.03"), "{data}");
    assert_eq!(data["totals"]["sellPrincipal"], json!("40.05"), "{data}");
    assert_eq!(data["totals"]["paid"], json!("30.03"), "{data}");
    assert_eq!(data["totals"]["received"], json!("40.05"), "{data}");
    assert_eq!(data["totals"]["net"], json!("10.02"), "{data}");

    // Shares: 30.03 * 100 / 70.08 = 42.8510... -> 42.85
    //         40.05 * 100 / 70.08 = 57.1489... -> 57.15
    assert_eq!(data["totals"]["buyShare"], json!("42.85"), "{data}");
    assert_eq!(data["totals"]["sellShare"], json!("57.15"), "{data}");

    room.finish().await;
}

async fn save_midpoint_tx(
    room: &Room,
    branch: &str,
    side: &str,
    amount: &str,
    rate: &str,
    value_date: &str,
) -> String {
    let t = room.tick();
    let here = BranchId::new(branch);
    let person = ReserveCustomer {
        name: "Somchai Jaidee".to_owned(),
        passport_no: "AA1234567".to_owned(),
        nationality: "THA".to_owned(),
        birth_date: String::new(),
        passport_expiry: String::new(),
        mode: "LIVE".to_owned(),
    };
    let customer = CustomerService::new(room.desk.db.branch_tx())
        .reserve(&here, person.clone(), t)
        .await
        .expect("reserve the customer")
        .customer_id;
    let passport = room
        .desk
        .files
        .upload(&here, "owner", "image/jpeg", JPEG, t)
        .await
        .expect("upload a passport");
    let cmd = SaveTransaction {
        customer: person,
        submission_id: format!("submission-{branch}-{:012}", next()),
        customer_id: customer,
        side: side.to_owned(),
        currency: "USD".to_owned(),
        amount: amount.to_owned(),
        rate: rate.to_owned(),
        fee: "0".to_owned(),
        bank_fee: "0".to_owned(),
        method: "Cash".to_owned(),
        payment_ref: String::new(),
        account: "Cash drawer 1".to_owned(),
        staff: "EMP-001".to_owned(),
        booking: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: value_date.to_owned(),
        paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
        payment_confirmed: true,
        identity_confirmed: true,
        passport: Some(passport),
        slip: None,
        reference_rate: None,
    };
    room.desk
        .workflow
        .save(&here, cmd, "owner", t)
        .await
        .unwrap_or_else(|e| panic!("save an exchange in {branch}: {e:?}"))
        .document_ref
}
