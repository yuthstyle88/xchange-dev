//! Ticket #469 (V2-reports, `docs/design/xchange-v3-web.md` §4.2, §5.1, §12.1, §12.8, §12.14): the
//! bounded view the admin reports menu reads. Written before the implementation by the lane
//! controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/admin_reports?branch=&since=&from=&to=&staff=&staffQ=` — a `BranchView`
//!   inheriting `(EntryPath::Rpc, "adminSnapshot")`, filtered by the period (`from` / `to`, design
//!   §12.1: ISO days, both inclusive, `''` or absent = open on that side; `from > to` →
//!   `400 periodOrder`, `field: "to"`; a malformed day → `400 invalidRequest` naming it) and, for
//!   the employee tab, by `staff` (exact) and `staffQ` (substring, case-insensitive, literal).
//! - `data` = `{period, stats, byChannel, byCurrency, reference, byEmployee, employeeTotals}`, every
//!   figure computed by the server over the period's LIVE exchanges of the branch (W7). Money is a
//!   decimal string with two places (`"0.00"` when there is none, ticket §9); counts are JSON
//!   numbers. v3 stores an exchange only once it is saved, so every stored exchange is COMPLETED
//!   (`snapshot.rs:15-16`): the pending and cancelled counts are 0.
//!   - `stats` (`admin-html:69`): `received` (Σ settlement of SELL), `paid` (Σ settlement of BUY),
//!     `fees` (Σ customer fee), `bankFees` (Σ bank fee) — v2's `totals` (`admin-html:60`).
//!   - `byChannel` (`admin-html:70`): always the five channels in v2's order `Cash`,
//!     `Bank Transfer`, `Credit Card`, `Alipay`, `PromptPay`, each `{method, count, received, paid,
//!     net}`; a stored `SCB Transfer` is counted as `Bank Transfer` (ticket §9;
//!     `employee-summary.js:5`; the snapshot already answers it so, `snapshot.rs:47`).
//!   - `byCurrency` (`admin-html:71`): one row per currency that has an exchange in the period, in
//!     the order of the branch's rate list (`state.rates`, which v3 answers in currency-code order,
//!     `xc-rate` `RateService::current`), each `{currency, buyAmount, sellAmount, principal, fees}`.
//!   - `reference` (`admin-html:103`, `:144`, `XCStock.valuation` `:36`): `{referenced,
//!     unreferenced, value, spread, percent}` — per exchange with a reference rate, the reference
//!     value `round2(amount × referenceRate)` and the spread (`BUY`: value − principal, `SELL`:
//!     principal − value); `percent` = Σspread / Σvalue × 100 to two places, `null` when no exchange
//!     has a reference rate (v2 then prints `ยังไม่มีเรทอ้างอิง`).
//!   - `byEmployee` (`employee-summary-core.js`): one group per employee name of the period, sorted
//!     by name, NOT narrowed by `staff` / `staffQ` (the select lists every group,
//!     `employee-summary.js:11`; design §12.14 "the name list in the select is the bounded byEmployee
//!     answer"); each `{staff, count, completed, pending, cancelled, received, paid, net, fees,
//!     bankFees, methods, currencies}`, `methods` the five channels in the summary's order `Cash`,
//!     `Credit Card`, `Alipay`, `PromptPay`, `Bank Transfer` (`employee-summary-core.js:3`) each
//!     `{method, count, received, paid, net}`, `currencies` in the order the employee first handled
//!     them (`:16`), each `{currency, received, paid}` in units of that currency (BUY → received,
//!     SELL → paid).
//!   - `employeeTotals` (`employee-summary.js:8-11`): `{people, completed, received, paid, net}` over
//!     the groups `staff` and `staffQ` select — the line `รวม … คน · สำเร็จ … รายการ · รับเข้า … ·
//!     จ่ายออก … · สุทธิ … THB`.
//!
//!   `staff` / `staffQ` change nothing but `employeeTotals`.
//! - LIVE only (design §12.5): a DEMO exchange is in nothing the view answers.
//! - §4.2: the one table the view reads, `transactions`, has one writer, the exchange save
//!   (`xc-exchange` `transaction_repository.rs` `insert_in`, inside `BranchTx`): one save, then the
//!   same `since` answers `changed: true`.
//!
//! Field names are [proposed] by the ticket ("`stats`, `byChannel`, `byCurrency`, `byEmployee`") and
//! fixed here, with the sub-fields above.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`); exchanges are saved through the real
//! `SaveTransactionWorkflow` behind the routes, as `v3_admin_parts_views_test.rs` does. The test's
//! clock is advanced before every write, so no two exchanges share a record time; no assertion
//! depends on the record time.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`) for the view tests; nothing skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
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
const REPORTS: &str = "admin_reports";

const MAIN: &str = "MAIN";
const B9: &str = "B9";

/// The counter rates of every branch of the room, `(code, buy, sell)`.
const RATES: [(&str, &str, &str); 4] = [
    ("USD", "30", "40"),
    ("EUR", "35", "38"),
    ("JPY", "0.2", "0.25"),
    ("GBP", "40", "45"),
];

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

const DATA_FIELDS: [&str; 7] = [
    "period",
    "stats",
    "byChannel",
    "byCurrency",
    "reference",
    "byEmployee",
    "employeeTotals",
];
const STATS_FIELDS: [&str; 4] = ["received", "paid", "fees", "bankFees"];
const CHANNEL_FIELDS: [&str; 5] = ["method", "count", "received", "paid", "net"];
const CURRENCY_FIELDS: [&str; 5] = ["currency", "buyAmount", "sellAmount", "principal", "fees"];
const REFERENCE_FIELDS: [&str; 5] = ["referenced", "unreferenced", "value", "spread", "percent"];
const EMPLOYEE_FIELDS: [&str; 12] = [
    "staff",
    "count",
    "completed",
    "pending",
    "cancelled",
    "received",
    "paid",
    "net",
    "fees",
    "bankFees",
    "methods",
    "currencies",
];
const EMPLOYEE_CURRENCY_FIELDS: [&str; 3] = ["currency", "received", "paid"];
const TOTALS_FIELDS: [&str; 5] = ["people", "completed", "received", "paid", "net"];

/// `admin-html:70`: the payment report's five rows, in this order.
const CHANNELS: [&str; 5] = [
    "Cash",
    "Bank Transfer",
    "Credit Card",
    "Alipay",
    "PromptPay",
];
/// `employee-summary-core.js:3`: an employee's five channels, in this order.
const EMPLOYEE_CHANNELS: [&str; 5] = [
    "Cash",
    "Credit Card",
    "Alipay",
    "PromptPay",
    "Bank Transfer",
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
        format!("/api/v3/views/{REPORTS}")
    } else {
        format!("/api/v3/views/{REPORTS}?{query}")
    }
}

/// One exchange to save.
#[derive(Clone, Copy)]
struct Ex {
    side: &'static str,
    currency: &'static str,
    amount: &'static str,
    fee: &'static str,
    bank_fee: &'static str,
    method: &'static str,
    staff: &'static str,
    reference: Option<&'static str>,
    value_date: &'static str,
    live: bool,
}

impl Ex {
    fn buy(amount: &'static str, fee: &'static str, value_date: &'static str) -> Ex {
        Ex {
            side: "BUY",
            currency: "USD",
            amount,
            fee,
            bank_fee: "0",
            method: "Cash",
            staff: "dao",
            reference: None,
            value_date,
            live: true,
        }
    }

    fn sell(amount: &'static str, fee: &'static str, value_date: &'static str) -> Ex {
        Ex {
            side: "SELL",
            ..Ex::buy(amount, fee, value_date)
        }
    }

    fn of(self, currency: &'static str) -> Ex {
        Ex { currency, ..self }
    }

    fn by(self, method: &'static str) -> Ex {
        Ex { method, ..self }
    }

    fn staff(self, staff: &'static str) -> Ex {
        Ex { staff, ..self }
    }

    fn bank_fee(self, bank_fee: &'static str) -> Ex {
        Ex { bank_fee, ..self }
    }

    fn reference(self, reference: &'static str) -> Ex {
        Ex {
            reference: Some(reference),
            ..self
        }
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
    /// `MAIN`, all signed in; the counter rates of [`RATES`] in both branches; an open shift in
    /// both, so LIVE exchanges can be saved.
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
            for (code, buy, sell) in RATES {
                room.set_rate(branch, code, buy, sell).await;
            }
            room.open_shift(branch).await;
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

    async fn ok(&self, who: &str, action: &str, payload: Value) -> Value {
        self.tick();
        let call = Call::action(ORIGIN, RPC, action, payload.clone()).cookie(&self.cookie(who));
        let answer = self.app.send(call).await;
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
                    "opening": { "THB": 1000000, "USD": 100000, "EUR": 100000, "JPY": 10000000 } }),
        )
        .await;
    }

    /// One exchange saved through the real workflow behind the routes, at the counter rate of its
    /// side and currency; a non-cash one carries a payment reference and a slip. Its document ref.
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
        let (_, buy, sell) = RATES
            .iter()
            .find(|(code, _, _)| *code == ex.currency)
            .copied()
            .unwrap_or_else(|| panic!("a counter rate for {}", ex.currency));
        let rate = if ex.side == "BUY" { buy } else { sell };
        let (payment_ref, slip) = if ex.method == "Cash" {
            (String::new(), None)
        } else {
            let slip = self
                .desk
                .files
                .upload(&here, "owner", "image/jpeg", JPEG, t)
                .await
                .expect("upload a slip");
            (format!("PAY-{:06}", next()), Some(slip))
        };
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer,
            side: ex.side.to_owned(),
            currency: ex.currency.to_owned(),
            amount: ex.amount.to_owned(),
            rate: rate.to_owned(),
            fee: ex.fee.to_owned(),
            bank_fee: ex.bank_fee.to_owned(),
            method: ex.method.to_owned(),
            payment_ref,
            account: "Cash drawer 1".to_owned(),
            staff: ex.staff.to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: ex.value_date.to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip,
            reference_rate: ex.reference.map(str::to_owned),
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

/// A channel row as the wire carries it: money exactly two places, the count a number.
fn channel(method: &str, count: u64, received: &str, paid: &str, net: &str) -> Value {
    json!({ "method": method, "count": count, "received": received, "paid": paid, "net": net })
}

fn zero_channels(order: &[&str]) -> Value {
    Value::Array(
        order
            .iter()
            .map(|m| channel(m, 0, "0.00", "0.00", "0.00"))
            .collect(),
    )
}

fn names(groups: &Value) -> Vec<String> {
    groups
        .as_array()
        .unwrap_or_else(|| panic!("byEmployee is a list: {groups}"))
        .iter()
        .map(|g| {
            g["staff"]
                .as_str()
                .unwrap_or_else(|| panic!("a group's staff is a string: {g}"))
                .to_owned()
        })
        .collect()
}

fn group<'a>(data: &'a Value, staff: &str) -> &'a Value {
    data["byEmployee"]
        .as_array()
        .unwrap_or_else(|| panic!("byEmployee is a list: {data}"))
        .iter()
        .find(|g| g["staff"] == json!(staff))
        .unwrap_or_else(|| panic!("a group for {staff}: {data}"))
}

/// The parts of `data` that `staff` / `staffQ` must never change.
fn period_wide(data: &Value) -> Value {
    json!({
        "period": data["period"],
        "stats": data["stats"],
        "byChannel": data["byChannel"],
        "byCurrency": data["byCurrency"],
        "reference": data["reference"],
        "byEmployee": data["byEmployee"],
    })
}

// ── who may read it ──────────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_reports_answers_exactly_whom_the_snapshot_row_allows() {
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
        let what = format!("{REPORTS} as {who:?}");
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

// ── the answer's shape and an empty period (ticket §9 row 1) ─────────────────

#[actix_web::test]
async fn an_empty_period_answers_zeros_the_five_channels_and_empty_lists() {
    let room = Room::open().await;
    room.save(MAIN, Ex::buy("10", "1", "2026-10-06")).await; // outside the period asked
    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-01&to=2026-10-05")
        .await;
    assert_eq!(keys(&data), set_of(&DATA_FIELDS), "{data}");
    assert_eq!(
        data["period"],
        json!({ "from": "2026-10-01", "to": "2026-10-05" }),
        "the period answered"
    );
    assert_eq!(keys(&data["stats"]), set_of(&STATS_FIELDS), "{data}");
    assert_eq!(
        data["stats"],
        json!({ "received": "0.00", "paid": "0.00", "fees": "0.00", "bankFees": "0.00" }),
        "zeros from the server, as two-place decimal strings"
    );
    assert_eq!(
        data["byChannel"],
        zero_channels(&CHANNELS),
        "the five rows of admin-html:70 are always there, in v2's order"
    );
    assert_eq!(data["byCurrency"], json!([]), "no currency row: {data}");
    assert_eq!(
        keys(&data["reference"]),
        set_of(&REFERENCE_FIELDS),
        "{data}"
    );
    assert_eq!(
        data["reference"],
        json!({ "referenced": 0, "unreferenced": 0, "value": "0.00", "spread": "0.00",
                "percent": null }),
        "no reference rate in the period: percent is null (ยังไม่มีเรทอ้างอิง)"
    );
    assert_eq!(data["byEmployee"], json!([]), "{data}");
    assert_eq!(
        keys(&data["employeeTotals"]),
        set_of(&TOTALS_FIELDS),
        "{data}"
    );
    assert_eq!(
        data["employeeTotals"],
        json!({ "people": 0, "completed": 0, "received": "0.00", "paid": "0.00", "net": "0.00" }),
        "{data}"
    );

    // no period named: open on both sides, answered as ''
    let data = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(data["period"], json!({ "from": "", "to": "" }), "{data}");
    assert_eq!(
        data["stats"]["paid"],
        json!("299.00"),
        "the open period has it: {data}"
    );
    room.finish().await;
}

// ── stats and channels (P1) ──────────────────────────────────────────────────

/// `admin-html:60`, `:69`, `:70`: the four stat figures and the five channel rows over the
/// period's LIVE exchanges of the branch; `SCB Transfer` is a Bank Transfer (ticket §9 row 2).
#[actix_web::test]
async fn stats_and_channels_are_v2s_sums_over_the_periods_live_exchanges() {
    let room = Room::open().await;
    // in MAIN, inside 2026-10-05 … 2026-10-06
    room.save(MAIN, Ex::buy("100", "50", "2026-10-05")).await; // Cash, paid 2950.00
    room.save(MAIN, Ex::buy("10", "0", "2026-10-06").by("Bank Transfer"))
        .await; // paid 300.00
    room.save(
        MAIN,
        Ex::sell("27.5", "20", "2026-10-06")
            .by("SCB Transfer")
            .bank_fee("15"),
    )
    .await; // received 1120.00
    room.save(MAIN, Ex::sell("1", "0", "2026-10-06").by("PromptPay"))
        .await; // received 40.00
    room.save(
        MAIN,
        Ex::buy("2", "1", "2026-10-05")
            .by("Credit Card")
            .bank_fee("2.5"),
    )
    .await; // paid 59.00
    // none of these may count
    room.save(MAIN, Ex::buy("1", "0", "2026-10-04")).await; // before the period
    room.save(MAIN, Ex::sell("1", "0", "2026-10-07").by("Alipay"))
        .await; // after the period
    room.save(MAIN, Ex::buy("500", "10", "2026-10-06").demo())
        .await; // DEMO
    room.save(B9, Ex::sell("5", "1", "2026-10-06").by("Alipay"))
        .await; // another branch

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    assert_eq!(
        data["stats"],
        json!({ "received": "1160.00", "paid": "3309.00", "fees": "71.00", "bankFees": "17.50" }),
        "admin-html:69 over admin-html:60's totals"
    );
    assert_eq!(
        data["byChannel"],
        json!([
            channel("Cash", 1, "0.00", "2950.00", "-2950.00"),
            channel("Bank Transfer", 2, "1120.00", "300.00", "820.00"),
            channel("Credit Card", 1, "0.00", "59.00", "-59.00"),
            channel("Alipay", 0, "0.00", "0.00", "0.00"),
            channel("PromptPay", 1, "40.00", "0.00", "40.00"),
        ]),
        "admin-html:70: five rows in v2's order, SCB Transfer inside Bank Transfer"
    );
    for row in data["byChannel"].as_array().expect("a list") {
        assert_eq!(keys(row), set_of(&CHANNEL_FIELDS), "{row}");
    }
    assert!(
        !data.to_string().contains("SCB Transfer"),
        "the old method name is never answered: {data}"
    );
    room.finish().await;
}

// ── currencies (P1) ──────────────────────────────────────────────────────────

/// `admin-html:71`: a row per currency that has an exchange in the period, in the order of the
/// branch's rate list (currency-code order), with what the shop bought and sold of it in its own
/// units, the principal in THB and the customer fees.
#[actix_web::test]
async fn currencies_are_the_periods_in_code_order_with_v2s_sums() {
    let room = Room::open().await;
    // saved USD, JPY, EUR: neither the save order nor the amounts give the code order
    room.save(MAIN, Ex::buy("100", "5", "2026-10-06")).await; // USD 3000.00
    room.save(MAIN, Ex::sell("10", "2", "2026-10-06")).await; // USD 400.00
    room.save(MAIN, Ex::buy("10000", "0", "2026-10-06").of("JPY"))
        .await; // JPY 2000.00
    room.save(MAIN, Ex::sell("20", "3", "2026-10-05").of("EUR"))
        .await; // EUR 760.00
    room.save(MAIN, Ex::buy("1", "0", "2026-10-01").of("EUR"))
        .await; // outside the period
    room.save(MAIN, Ex::sell("1", "0", "2026-10-06").of("GBP").demo())
        .await; // DEMO: GBP has no row

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    assert_eq!(
        data["byCurrency"],
        json!([
            { "currency": "EUR", "buyAmount": "0.00", "sellAmount": "20.00",
              "principal": "760.00", "fees": "3.00" },
            { "currency": "JPY", "buyAmount": "10000.00", "sellAmount": "0.00",
              "principal": "2000.00", "fees": "0.00" },
            { "currency": "USD", "buyAmount": "100.00", "sellAmount": "10.00",
              "principal": "3400.00", "fees": "7.00" },
        ]),
        "admin-html:71, in the rate list's order"
    );
    for row in data["byCurrency"].as_array().expect("a list") {
        assert_eq!(keys(row), set_of(&CURRENCY_FIELDS), "{row}");
    }
    room.finish().await;
}

// ── the reference comparison (P1) ────────────────────────────────────────────

/// `admin-html:103`, `:144` with `XCStock.valuation` (`:36`): the period's completed exchanges
/// with a reference rate, their reference value and spread from the shop's side, the percent of
/// the spread to the value; the others counted as not yet given.
#[actix_web::test]
async fn the_reference_comparison_sums_the_referenced_exchanges_of_the_period() {
    let room = Room::open().await;
    // BUY 100 @ 30, reference 30.5: value 3050.00, spread 3050.00 − 3000.00 = 50.00
    room.save(MAIN, Ex::buy("100", "5", "2026-10-06").reference("30.5"))
        .await;
    // SELL 27.5 @ 40, reference 39.333: value 1081.6575 → 1081.66, spread 1100.00 − 1081.66 = 18.34
    room.save(
        MAIN,
        Ex::sell("27.5", "0", "2026-10-06").reference("39.333"),
    )
    .await;
    // SELL 2 @ 40, reference 39.003: value 78.006 → 78.01, spread 80.00 − 78.01 = 1.99; rounded per
    // exchange, as XCStock.valuation does: rounding only the sum would give 4209.66, not 4209.67
    room.save(MAIN, Ex::sell("2", "0", "2026-10-06").reference("39.003"))
        .await;
    // no reference rate: counted as not given
    room.save(MAIN, Ex::buy("10", "0", "2026-10-06")).await;
    room.save(MAIN, Ex::sell("1", "0", "2026-10-06").of("EUR"))
        .await;
    // outside the period, with a reference rate
    room.save(MAIN, Ex::buy("100", "0", "2026-10-01").reference("31"))
        .await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    // 70.33 / 4209.67 × 100 = 1.670677… → 1.67
    assert_eq!(
        data["reference"],
        json!({ "referenced": 3, "unreferenced": 2, "value": "4209.67", "spread": "70.33",
                "percent": "1.67" }),
        "{data}"
    );

    // a SELL below the reference: the spread is negative, and so is the percent
    room.save(MAIN, Ex::sell("100", "0", "2026-10-06").reference("41.2"))
        .await; // value 4120.00, spread 4000.00 − 4120.00 = −120.00
    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    // −49.67 / 8329.67 × 100 = −0.596301… → −0.60
    assert_eq!(
        data["reference"],
        json!({ "referenced": 4, "unreferenced": 2, "value": "8329.67", "spread": "-49.67",
                "percent": "-0.60" }),
        "{data}"
    );
    room.finish().await;
}

// ── the employee summary (P3) ────────────────────────────────────────────────

async fn employees_room() -> Room {
    let room = Room::open().await;
    room.save(MAIN, Ex::buy("100", "5", "2026-10-06").staff("dao"))
        .await; // Cash, paid 2995.00
    room.save(
        MAIN,
        Ex::sell("10", "2", "2026-10-06")
            .by("PromptPay")
            .staff("anan"),
    )
    .await; // received 402.00
    room.save(
        MAIN,
        Ex::sell("20", "3", "2026-10-05")
            .of("EUR")
            .by("SCB Transfer")
            .bank_fee("1")
            .staff("dao"),
    )
    .await; // received 763.00
    room.save(
        MAIN,
        Ex::buy("10000", "0", "2026-10-06")
            .of("JPY")
            .by("Credit Card")
            .staff("somchai"),
    )
    .await; // paid 2000.00
    room.save(MAIN, Ex::buy("1", "0", "2026-10-06").staff("dao"))
        .await; // Cash, paid 30.00
    room.save(
        MAIN,
        Ex::buy("2", "1", "2026-10-05")
            .by("Alipay")
            .bank_fee("0.5")
            .staff("anan"),
    )
    .await; // paid 59.00
    // none of these may count
    room.save(MAIN, Ex::buy("50", "0", "2026-10-01").staff("dao"))
        .await;
    room.save(MAIN, Ex::buy("50", "0", "2026-10-06").staff("zed").demo())
        .await;
    room.save(B9, Ex::buy("50", "0", "2026-10-06").staff("dao"))
        .await;
    room
}

const EMPLOYEES: &str = "branch=MAIN&from=2026-10-05&to=2026-10-06";

/// `employee-summary-core.js`: one group per employee of the period, sorted by name, with v2's
/// figures; each group's five channels in the summary's order and its currencies in the order the
/// employee first handled them.
#[actix_web::test]
async fn by_employee_groups_v2s_figures_per_employee_sorted_by_name() {
    let room = employees_room().await;
    let data = room.data("OWNER", EMPLOYEES).await;
    assert_eq!(
        names(&data["byEmployee"]),
        vec!["anan", "dao", "somchai"],
        "sorted by name, the DEMO-only and out-of-period names absent: {data}"
    );
    for g in data["byEmployee"].as_array().expect("a list") {
        assert_eq!(keys(g), set_of(&EMPLOYEE_FIELDS), "{g}");
        let order: Vec<&str> = g["methods"]
            .as_array()
            .expect("methods is a list")
            .iter()
            .map(|m| m["method"].as_str().expect("a method name"))
            .collect();
        assert_eq!(
            order, EMPLOYEE_CHANNELS,
            "employee-summary-core.js:3: every employee has the five channels in this order: {g}"
        );
        for m in g["methods"].as_array().expect("methods is a list") {
            assert_eq!(keys(m), set_of(&CHANNEL_FIELDS), "{m}");
        }
        for c in g["currencies"].as_array().expect("currencies is a list") {
            assert_eq!(keys(c), set_of(&EMPLOYEE_CURRENCY_FIELDS), "{c}");
        }
    }

    let dao = group(&data, "dao");
    assert_eq!(
        (
            &dao["count"],
            &dao["completed"],
            &dao["pending"],
            &dao["cancelled"]
        ),
        (&json!(3), &json!(3), &json!(0), &json!(0)),
        "{dao}"
    );
    assert_eq!(
        (
            &dao["received"],
            &dao["paid"],
            &dao["net"],
            &dao["fees"],
            &dao["bankFees"]
        ),
        (
            &json!("763.00"),
            &json!("3025.00"),
            &json!("-2262.00"),
            &json!("8.00"),
            &json!("1.00")
        ),
        "{dao}"
    );
    assert_eq!(
        dao["methods"],
        json!([
            channel("Cash", 2, "0.00", "3025.00", "-3025.00"),
            channel("Credit Card", 0, "0.00", "0.00", "0.00"),
            channel("Alipay", 0, "0.00", "0.00", "0.00"),
            channel("PromptPay", 0, "0.00", "0.00", "0.00"),
            channel("Bank Transfer", 1, "763.00", "0.00", "763.00"),
        ]),
        "employee-summary-core.js:3, :11-15; SCB Transfer is a Bank Transfer"
    );
    assert_eq!(
        dao["currencies"],
        json!([
            { "currency": "USD", "received": "101.00", "paid": "0.00" },
            { "currency": "EUR", "received": "0.00", "paid": "20.00" },
        ]),
        "employee-summary-core.js:16: BUY → received, SELL → paid, in units; first handled first"
    );

    let anan = group(&data, "anan");
    assert_eq!(
        (&anan["count"], &anan["completed"]),
        (&json!(2), &json!(2)),
        "{anan}"
    );
    assert_eq!(
        (
            &anan["received"],
            &anan["paid"],
            &anan["net"],
            &anan["fees"],
            &anan["bankFees"]
        ),
        (
            &json!("402.00"),
            &json!("59.00"),
            &json!("343.00"),
            &json!("3.00"),
            &json!("0.50")
        ),
        "{anan}"
    );
    assert_eq!(
        anan["methods"],
        json!([
            channel("Cash", 0, "0.00", "0.00", "0.00"),
            channel("Credit Card", 0, "0.00", "0.00", "0.00"),
            channel("Alipay", 1, "0.00", "59.00", "-59.00"),
            channel("PromptPay", 1, "402.00", "0.00", "402.00"),
            channel("Bank Transfer", 0, "0.00", "0.00", "0.00"),
        ]),
        "{anan}"
    );
    assert_eq!(
        anan["currencies"],
        json!([{ "currency": "USD", "received": "2.00", "paid": "10.00" }]),
        "{anan}"
    );

    let somchai = group(&data, "somchai");
    assert_eq!(
        (&somchai["received"], &somchai["paid"], &somchai["net"]),
        (&json!("0.00"), &json!("2000.00"), &json!("-2000.00")),
        "{somchai}"
    );
    assert_eq!(
        somchai["currencies"],
        json!([{ "currency": "JPY", "received": "10000.00", "paid": "0.00" }]),
        "{somchai}"
    );

    assert_eq!(
        data["employeeTotals"],
        json!({ "people": 3, "completed": 6, "received": "1165.00", "paid": "5084.00",
                "net": "-3919.00" }),
        "employee-summary.js:11 with no employee picked: everyone"
    );
    room.finish().await;
}

/// `employee-summary.js:8-11`, design §12.14: `staff` (exact) and `staffQ` (substring, case
/// folded) pick the employees whose totals the line shows; the groups and everything else stay the
/// period's.
#[actix_web::test]
async fn staff_and_staff_q_narrow_only_the_employee_totals() {
    let room = employees_room().await;
    let all = room.data("OWNER", EMPLOYEES).await;
    for (filter, totals) in [
        (
            "staff=dao",
            json!({ "people": 1, "completed": 3, "received": "763.00", "paid": "3025.00",
                    "net": "-2262.00" }),
        ),
        (
            "staff=da",
            json!({ "people": 0, "completed": 0, "received": "0.00", "paid": "0.00",
                    "net": "0.00" }),
        ),
        (
            "staffQ=O",
            json!({ "people": 2, "completed": 4, "received": "763.00", "paid": "5025.00",
                    "net": "-4262.00" }),
        ),
        (
            "staffQ=an",
            json!({ "people": 1, "completed": 2, "received": "402.00", "paid": "59.00",
                    "net": "343.00" }),
        ),
        (
            "staff=dao&staffQ=A",
            json!({ "people": 1, "completed": 3, "received": "763.00", "paid": "3025.00",
                    "net": "-2262.00" }),
        ),
        (
            "staff=dao&staffQ=x",
            json!({ "people": 0, "completed": 0, "received": "0.00", "paid": "0.00",
                    "net": "0.00" }),
        ),
        (
            "staff=&staffQ=",
            json!({ "people": 3, "completed": 6, "received": "1165.00", "paid": "5084.00",
                    "net": "-3919.00" }),
        ),
    ] {
        let data = room.data("OWNER", &format!("{EMPLOYEES}&{filter}")).await;
        assert_eq!(data["employeeTotals"], totals, "{filter}: {data}");
        assert_eq!(
            period_wide(&data),
            period_wide(&all),
            "{filter}: nothing but employeeTotals moves"
        );
    }
    room.finish().await;
}

/// v2 uses `includes` (`employee-summary.js:8`): `staffQ` is a literal substring — `%`, `_` and
/// `\` are not patterns (design §12.1).
#[actix_web::test]
async fn staff_q_is_a_literal_substring() {
    let room = Room::open().await;
    room.save(MAIN, Ex::buy("1", "0", "2026-10-06").staff("a_b"))
        .await;
    room.save(MAIN, Ex::buy("2", "0", "2026-10-06").staff("axb"))
        .await;
    room.save(MAIN, Ex::buy("4", "0", "2026-10-06").staff("c\\d"))
        .await;
    for (q, people, paid) in [
        ("_", 1, "30.00"),
        ("a_", 1, "30.00"),
        ("%25", 0, "0.00"),
        ("a%25b", 0, "0.00"),
        ("%5C", 1, "120.00"),
        ("X", 1, "60.00"),
        ("", 3, "210.00"),
    ] {
        let data = room.data("OWNER", &format!("branch=MAIN&staffQ={q}")).await;
        assert_eq!(
            (
                &data["employeeTotals"]["people"],
                &data["employeeTotals"]["paid"]
            ),
            (&json!(people), &json!(paid)),
            "staffQ={q}: {data}"
        );
    }
    room.finish().await;
}

// ── the period ───────────────────────────────────────────────────────────────

/// Design §12.1: both ends inclusive, `''` or absent = open on that side, by the exchange's date.
#[actix_web::test]
async fn the_period_is_inclusive_at_both_ends_and_open_where_empty() {
    let room = Room::open().await;
    room.save(MAIN, Ex::buy("1", "0", "2026-10-01")).await; // paid 30
    room.save(MAIN, Ex::buy("3", "0", "2026-10-03")).await; // paid 90
    room.save(MAIN, Ex::buy("5", "0", "2026-10-05")).await; // paid 150

    for (query, paid, count) in [
        ("from=2026-10-03&to=2026-10-03", "90.00", 1),
        ("from=2026-10-03", "240.00", 2),
        ("from=2026-10-03&to=", "240.00", 2),
        ("to=2026-10-03", "120.00", 2),
        ("from=&to=2026-10-03", "120.00", 2),
        ("from=2026-10-02&to=2026-10-04", "90.00", 1),
        ("from=2026-10-01&to=2026-10-05", "270.00", 3),
        ("", "270.00", 3),
        ("from=2026-10-06&to=2026-10-09", "0.00", 0),
    ] {
        let q = if query.is_empty() {
            "branch=MAIN".to_string()
        } else {
            format!("branch=MAIN&{query}")
        };
        let data = room.data("OWNER", &q).await;
        assert_eq!(data["stats"]["paid"], json!(paid), "{query:?}: {data}");
        assert_eq!(
            data["byChannel"][0]["count"],
            json!(count),
            "{query:?}: {data}"
        );
        assert_eq!(
            data["employeeTotals"]["completed"],
            json!(count),
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
        "branch=MAIN&from=2026-10-07&to=2026-10-06&staff=dao&staffQ=a".to_string(),
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

// ── branches ─────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_owner_reads_the_branch_named_and_staff_their_own() {
    let room = Room::open().await;
    room.save(B9, Ex::sell("10", "0", "2026-10-06").staff("nine"))
        .await;

    let data = room.data("OWNER", "branch=B9").await;
    assert_eq!(data["stats"]["received"], json!("400.00"), "{data}");
    assert_eq!(names(&data["byEmployee"]), vec!["nine"], "{data}");

    for (who, query) in [
        ("OWNER", "branch=MAIN"),
        ("OWNER", ""),
        ("FRONT", "branch=B9"),
    ] {
        let data = room.data(who, query).await;
        assert_eq!(
            data["stats"]["received"],
            json!("0.00"),
            "{who} {query:?}: MAIN's: {data}"
        );
        assert_eq!(data["byEmployee"], json!([]), "{who} {query:?}: {data}");
        assert_eq!(data["byCurrency"], json!([]), "{who} {query:?}: {data}");
    }
    room.finish().await;
}

// ── §4.2: the writer moves the version ───────────────────────────────────────

/// The one writer of the table the view reads, the exchange save, goes through `BranchTx`: the
/// same `since` then answers `changed: true` with the exchange in it; before it, an equal `since`
/// is exactly unchanged, whatever the filter.
#[actix_web::test]
async fn the_exchange_save_moves_the_version_the_reports_answer() {
    let room = Room::open().await;
    let q =
        |since: i64| format!("branch=MAIN&from=2026-10-01&to=2026-10-31&staffQ=da&since={since}");

    let v = room.version(MAIN).await;
    assert_eq!(
        room.view("OWNER", &q(v)).await,
        json!({ "version": v, "changed": false }),
        "an equal since: exactly the version and changed: false"
    );

    let mut since = v;
    for n in 1..=2u64 {
        room.save(MAIN, Ex::buy("10", "0", "2026-10-06").staff("dao"))
            .await;
        let now = room.version(MAIN).await;
        assert!(
            now > since,
            "the save moved MAIN's version ({since} → {now})"
        );
        let body = room.view("OWNER", &q(since)).await;
        assert_eq!(body["version"], json!(now), "save {n}: {body}");
        assert_eq!(body["changed"], json!(true), "save {n}: {body}");
        assert_eq!(
            body["data"]["employeeTotals"]["completed"],
            json!(n),
            "save {n}: {body}"
        );
        assert_eq!(
            body["data"]["byChannel"][0]["count"],
            json!(n),
            "save {n}: {body}"
        );
        assert_eq!(
            room.view("OWNER", &q(now)).await,
            json!({ "version": now, "changed": false }),
            "after save {n}, the new version is unchanged"
        );
        since = now;
    }

    // a save in another branch leaves MAIN's version where it was
    room.save(B9, Ex::buy("10", "0", "2026-10-06")).await;
    assert_eq!(
        room.view("OWNER", &q(since)).await,
        json!({ "version": since, "changed": false }),
        "B9's save is not MAIN's"
    );
    room.finish().await;
}

// ── the generated types ──────────────────────────────────────────────────────

fn exported() -> BTreeMap<String, String> {
    let entry = VIEWS
        .iter()
        .find(|v| v.name == REPORTS)
        .unwrap_or_else(|| panic!("the view {REPORTS} is in the generated table"));
    let dir: PathBuf =
        std::env::temp_dir().join(format!("xc-469-{}-{}", std::process::id(), next()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    (entry.export_types)(&dir).unwrap_or_else(|e| panic!("export {REPORTS}: {e}"));
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
fn admin_reports_is_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert!(names.contains(&REPORTS), "{REPORTS} in {names:?}");
}

#[test]
fn admin_reports_types_carry_the_fields_money_as_strings_counts_as_numbers() {
    let files = exported();
    let ts = files
        .get("admin_reports.ts")
        .unwrap_or_else(|| panic!("admin_reports.ts in {:?}", files.keys()));
    for field in [
        "received",
        "paid",
        "net",
        "fees",
        "bankFees",
        "buyAmount",
        "sellAmount",
        "principal",
        "value",
        "spread",
    ] {
        has_field(ts, field, &["string", "Money"]);
    }
    has_field(
        ts,
        "percent",
        &["string | null", "Money | null", "Rate | null"],
    );
    for field in [
        "count",
        "completed",
        "pending",
        "cancelled",
        "referenced",
        "unreferenced",
        "people",
    ] {
        has_field(ts, field, &["number"]);
    }
    for field in ["method", "currency", "staff"] {
        has_field(ts, field, &["string"]);
    }
    for list in [
        "byChannel",
        "byCurrency",
        "byEmployee",
        "methods",
        "currencies",
    ] {
        assert!(
            ts.contains(&format!("{list}: Array<")),
            "`{list}` is a list in:\n{ts}"
        );
    }
    // the filter the menu sends: the period's keys and the employee keys
    for field in ["from", "to", "staff", "staffQ"] {
        has_field(ts, field, &["string"]);
    }
    assert!(!ts.contains("bigint"), "no bigint on the wire:\n{ts}");
    assert!(
        !ts.contains("transaction_row"),
        "the reports carry aggregates, not rows:\n{ts}"
    );
}

/// Decision 6: staffQ of 101 characters -> 400 invalidRequest, field: "staffQ"; 100 characters -> 200.
#[actix_web::test]
async fn staff_q_over_100_characters_is_refused_naming_staff_q() {
    let room = Room::open().await;
    let q100 = "a".repeat(100);
    let q101 = "a".repeat(101);

    // 100 characters is accepted
    let ok_query = format!("branch=MAIN&staffQ={q100}");
    let ok_answer = room.get("OWNER", &view_path(&ok_query)).await;
    assert_eq!(
        ok_answer.status, 200,
        "100 chars staffQ is ok: {}",
        ok_answer.text
    );

    // 101 characters is refused naming staffQ
    let bad_query = format!("branch=MAIN&staffQ={q101}");
    let bad_answer = room.get("OWNER", &view_path(&bad_query)).await;
    assert_eq!(
        bad_answer.status, 400,
        "101 chars staffQ is bad: {}",
        bad_answer.text
    );
    let body = bad_answer.json();
    assert_eq!(
        body["error"],
        json!({
            "code": "invalidRequest",
            "field": "staffQ",
            "message": "คำขอไม่ถูกต้อง",
        }),
        "{bad_query}: {body}"
    );

    room.finish().await;
}

/// Non-canonical dates (e.g. single digit month/day '2026-1-5') must be refused by Period::check.
#[actix_web::test]
async fn period_check_refuses_non_canonical_date() {
    let room = Room::open().await;
    for (query, field) in [
        ("branch=MAIN&from=2026-1-5", "from"),
        ("branch=MAIN&to=2026-1-5", "to"),
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

/// A reference value at an exact half-satang with an even preceding digit rounds half away from zero.
/// (10.5 * 0.05 = 0.525 -> "0.53", pinning mutant D1 MidpointNearestEven).
#[actix_web::test]
async fn reference_value_half_satang_midpoint_rounds_half_away_from_zero() {
    let room = Room::open().await;
    // SELL 10.5 USD @ 40, reference 0.05:
    // raw reference value = 10.5 * 0.05 = 0.525.
    // round_cents / half-away rounds 0.525 to 0.53.
    // banker's rounding (MidpointNearestEven) rounds to 0.52.
    room.save(MAIN, Ex::sell("10.5", "0", "2026-10-06").reference("0.05"))
        .await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    assert_eq!(
        data["reference"]["value"],
        json!("0.53"),
        "0.525 rounds to 0.53, not 0.52: {data}"
    );
    room.finish().await;
}

/// A reference percent at a midpoint rounds half away from zero, pinning mutant D3 MidpointNearestEven.
#[actix_web::test]
async fn reference_percent_midpoint_rounds_half_away_from_zero() {
    let room = Room::open().await;
    // SELL 4000 USD @ 40, reference 39.51:
    // value1 = 4000 * 39.51 = 158040.00, spread1 = 160000.00 - 158040.00 = 1960.00
    room.save(MAIN, Ex::sell("4000", "0", "2026-10-06").reference("39.51"))
        .await;
    // SELL 49 USD @ 40, reference 40.00:
    // value2 = 49 * 40.00 = 1960.00, spread2 = 1960.00 - 1960.00 = 0.00
    room.save(MAIN, Ex::sell("49", "0", "2026-10-06").reference("40.00"))
        .await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    // total_value = 160000.00, total_spread = 1960.00
    // percent = 1960.00 / 160000.00 * 100 = 1.225%
    // round_cents / half-away: 1.225 -> "1.23".
    // MidpointNearestEven: 1.225 -> "1.22".
    assert_eq!(
        data["reference"]["percent"],
        json!("1.23"),
        "1.225% rounds to 1.23, not 1.22: {data}"
    );
    room.finish().await;
}

/// A reference percent ending early is padded to two places (1.5% -> "1.50", pinning mutant D4 to_string()).
#[actix_web::test]
async fn reference_percent_padded_to_two_places_when_ending_early() {
    let room = Room::open().await;
    // SELL 100 USD @ 40, reference 39.40:
    // value1 = 3940.00, spread1 = 4000.00 - 3940.00 = 60.00
    room.save(MAIN, Ex::sell("100", "0", "2026-10-06").reference("39.40"))
        .await;
    // SELL 1.5 USD @ 40, reference 40.00:
    // value2 = 60.00, spread2 = 0.00
    room.save(MAIN, Ex::sell("1.5", "0", "2026-10-06").reference("40.00"))
        .await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    // total_value = 4000.00, total_spread = 60.00
    // percent = 60.00 / 4000.00 * 100 = 1.50%
    // fixed2 / format_money: "1.50".
    // to_string(): "1.5".
    assert_eq!(
        data["reference"]["percent"],
        json!("1.50"),
        "1.5% is padded to two places as 1.50, not 1.5: {data}"
    );
    room.finish().await;
}

/// Guard division by zero: total_value can round to 0 while referenced > 0.
#[actix_web::test]
async fn reference_comparison_guards_against_division_by_zero_when_total_value_rounds_to_zero() {
    let room = Room::open().await;
    // BUY 0.01 USD @ 30, reference 0.0001:
    // 0.01 * 0.0001 = 0.000001 -> rounds to 0.00 value.
    // referenced = 1, total_value = 0.00, spread = 0.00 - 0.30 = -0.30.
    room.save(MAIN, Ex::buy("0.01", "0", "2026-10-06").reference("0.0001"))
        .await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-05&to=2026-10-06")
        .await;
    assert_eq!(data["reference"]["referenced"], json!(1));
    assert_eq!(data["reference"]["value"], json!("0.00"));
    assert_eq!(data["reference"]["spread"], json!("-0.30"));
    assert!(
        data["reference"]["percent"].is_null(),
        "percent is null when total_value is 0.00: {data}"
    );
    room.finish().await;
}
