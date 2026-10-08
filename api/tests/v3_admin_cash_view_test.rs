//! Ticket #467 (V2-cash, `docs/design/xchange-v3-web.md` §4.2, §5.1, §12.1, §12.6, §12.9): the three
//! views the admin cash menu reads. Written before the implementation by the lane controller
//! (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Panel A5 (the valuation and profit calculators) is not this ticket's (controller,
//! issuecomment-6066362902): nothing here asks for it.
//!
//! - `GET /api/v3/views/admin_cash?branch=&since=&all=` — a `BranchView` inheriting
//!   `(EntryPath::Rpc, "adminSnapshot")`, its own filter `all` (`true` = every currency of the
//!   register, ticket §7 "and, on request, all"; a value that is not a boolean →
//!   `400 invalidRequest`, `field: "all"`). `data` = `{shift, stock, stats, reference, currencies,
//!   lastCounted}`:
//!   - `shift`: the open shift `{id, openedAt, staff}`, or `null` (`admin-html:101`);
//!   - `stock`: one row `{code, opening, net, expected}` per currency shown — THB, every currency
//!     the branch has a counter rate for, and every currency whose opening, net or expected is not
//!     zero (`admin-workspace.js:125-131`); with `all=true` every currency of the register. THB
//!     first, then A–Z (v2's register order, `admin-html:26`). `net = expected − opening`
//!     (`admin-html:105`). `[]` when no shift is open (`:105` shows the empty state);
//!   - `stats`: v2's `XCStock.cashFlow` (`admin-html:37`, printed by `:102`) of the open shift —
//!     `{opening, incoming, outgoing, remaining}` in THB; `null` when no shift is open;
//!   - `reference`: v2's `referenceSummary` (`admin-html:144`, called by `:103`) over the open
//!     shift's exchanges — `{withReference, withoutReference, spread, percent}`, the counts as JSON
//!     numbers, `spread` a two-place decimal string, `percent` a two-place decimal string (v2's
//!     `toFixed(2)`); no shift = no exchanges;
//!   - `currencies`: the register (153 codes), THB first then A–Z — the opening picker's and the
//!     movement dialog's choices (`admin-opening-currencies.js:23`, `admin-html:128`);
//!   - `lastCounted`: the prefill of the opening dialog (`admin-opening-currencies.js:9`,
//!     `:18-19`, `admin-html:127`): the last CLOSED shift's count — THB always, then each other
//!     currency counted above zero — as `[{code, amount}]`, or `null` when no shift has closed.
//!
//!   Every figure is a decimal string with two places, computed by the server (W7).
//! - `GET /api/v3/views/admin_cash_movements?branch=&before=&limit=` — a `PagedView`, `NoFilter`:
//!   the branch's stock movements, newest recorded first (`admin-html:106`), each
//!   `{id, created, shiftId, direction, currency, amount, reason, staff}`.
//! - `GET /api/v3/views/admin_shifts?branch=&before=&limit=` — a `PagedView`, `NoFilter`: the
//!   branch's shifts, newest opened first (`admin-html:107`), each
//!   `{id, status, openedAt, staff, note, difference, balances}`: `difference` is `null` for an
//!   open shift, else the non-zero `counted − expected` per currency as `[{code, amount}]` (`[]`
//!   when the count matched, `:107`); `balances` is the shift detail (`:130`): one row
//!   `{code, expected, counted, difference}` for THB and for every currency with a non-zero
//!   opening, expected, count or difference, THB first then A–Z — `counted` / `difference`
//!   `null` while the shift is open, `expected` the server's running figure.
//! - §4.2 / ticket §7: every writer of what the three read goes through `BranchTx` — shift open,
//!   stock move, shift close, the exchange save, abandon, and custody's shift effects — one write
//!   each, then the same `since` answers `changed: true`.
//!
//! Everything goes through the server's whole route table with real sessions (`rpc_common`);
//! writes are real `/api/rpc` actions, exchanges are saved through the real
//! `SaveTransactionWorkflow`, custody through the recorded custody day of `rpc/ops_actions.json`.
//! The test's clock is set before every write; record order is asserted by a clock that runs
//! backwards, so nothing here relies on two writes having different times.
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
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use compat_common::Fixture;
use rpc_common::{
    Answer, App, Call, Cookies, Desk, INVALID_REQUEST, Replay, Rule, one_of_each_role, sign_in,
};
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
use xc_kernel::money::CurrencyCode;
use xchange::presentation::http::v3::VIEWS;

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";
const CASH: &str = "admin_cash";
const MOVEMENTS: &str = "admin_cash_movements";
const SHIFTS: &str = "admin_shifts";
const VIEW_NAMES: [&str; 3] = [CASH, MOVEMENTS, SHIFTS];

const MAIN: &str = "MAIN";
const B9: &str = "B9";

/// The counter rates of USD in every branch of the room: the shop buys at 30, sells at 40.
const BUY_RATE: &str = "30";
const SELL_RATE: &str = "40";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

const DATA_FIELDS: [&str; 6] = [
    "shift",
    "stock",
    "stats",
    "reference",
    "currencies",
    "lastCounted",
];
const SHIFT_FIELDS: [&str; 3] = ["id", "openedAt", "staff"];
const STOCK_FIELDS: [&str; 4] = ["code", "opening", "net", "expected"];
const STATS_FIELDS: [&str; 4] = ["opening", "incoming", "outgoing", "remaining"];
const REFERENCE_FIELDS: [&str; 4] = ["withReference", "withoutReference", "spread", "percent"];
const MOVEMENT_FIELDS: [&str; 8] = [
    "id",
    "created",
    "shiftId",
    "direction",
    "currency",
    "amount",
    "reason",
    "staff",
];
const SHIFT_ROW_FIELDS: [&str; 7] = [
    "id",
    "status",
    "openedAt",
    "staff",
    "note",
    "difference",
    "balances",
];
const BALANCE_FIELDS: [&str; 4] = ["code", "expected", "counted", "difference"];

/// The custody day (`rpc_ops_test.rs` replays it against v2's answers; here it only writes).
const OPS_FILE: &str = "rpc/ops_actions.json";
const OPS_LAST: usize = 134;
const OPS_OWNERS_DOOR: std::ops::RangeInclusive<usize> = 9..=17;
const OPS_SWITCHED_ON_AT: usize = 15;
const OPS_UNKNOWN_NAME_BY_STATE: [usize; 3] = [2, 23, 40];
const OPS_HOLD_MADE_AT: usize = 105;
const OPS_HOLD_NAMED_AT: usize = 106;

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

fn list(value: &Value) -> &Vec<Value> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("a list: {value}"))
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal literal")
}

/// The register in v2's order: THB first, then the other codes A–Z (`admin-html:26`).
fn register() -> Vec<String> {
    let mut rest: Vec<String> = CurrencyCode::all()
        .iter()
        .map(|c| c.as_str().to_string())
        .filter(|c| c != "THB")
        .collect();
    rest.sort();
    let mut all = vec!["THB".to_string()];
    all.extend(rest);
    all
}

/// ISO-8601 in UTC with milliseconds, as every v2 time on the wire.
fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
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

/// `stock` as `code → (opening, net, expected)`, keeping the wire's strings.
fn stock_of(data: &Value) -> Vec<(String, String, String, String)> {
    list(&data["stock"])
        .iter()
        .map(|row| {
            assert_eq!(keys(row), set_of(&STOCK_FIELDS), "a stock row: {row}");
            let s = |k: &str| {
                row[k]
                    .as_str()
                    .unwrap_or_else(|| panic!("stock.{k} is a string on the wire (W7): {row}"))
                    .to_string()
            };
            (s("code"), s("opening"), s("net"), s("expected"))
        })
        .collect()
}

fn codes_of(data: &Value) -> Vec<String> {
    stock_of(data).into_iter().map(|r| r.0).collect()
}

fn ids(rows: &Value) -> Vec<String> {
    list(rows)
        .iter()
        .map(|row| {
            row["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row's id is a string: {row}"))
                .to_owned()
        })
        .collect()
}

/// One exchange of USD to save.
#[derive(Clone, Copy)]
struct Ex {
    side: &'static str,
    amount: &'static str,
    fee: &'static str,
    method: &'static str,
    reference: Option<&'static str>,
}

impl Ex {
    fn buy(amount: &'static str, fee: &'static str) -> Ex {
        Ex {
            side: "BUY",
            amount,
            fee,
            method: "Cash",
            reference: None,
        }
    }

    fn sell(amount: &'static str, fee: &'static str) -> Ex {
        Ex {
            side: "SELL",
            amount,
            fee,
            method: "Cash",
            reference: None,
        }
    }

    fn transfer(self) -> Ex {
        Ex {
            method: "Bank Transfer",
            ..self
        }
    }

    fn reference(self, rate: &'static str) -> Ex {
        Ex {
            reference: Some(rate),
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

    /// A view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, view: &str, who: &str, query: &str) -> Value {
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
    async fn data(&self, view: &str, who: &str, query: &str) -> Value {
        let body = self.view(view, who, query).await;
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

    async fn cash(&self, query: &str) -> Value {
        self.data(CASH, "OWNER", query).await
    }

    async fn post(&self, who: &str, action: &str, payload: Value) -> Answer {
        self.tick();
        self.post_now(who, action, payload).await
    }

    /// A write at the clock as it is set.
    async fn post_now(&self, who: &str, action: &str, payload: Value) -> Answer {
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

    /// The owner opens a shift in `branch` through `/api/rpc` with the opening given.
    async fn open_shift_with(&self, branch: &str, opening: Value) -> String {
        self.ok(
            "OWNER",
            "adminOpenShift",
            json!({ "branchId": branch, "requestId": request_id(), "opening": opening }),
        )
        .await;
        self.open_shift_id(branch).await
    }

    async fn open_shift(&self, branch: &str) -> String {
        self.open_shift_with(branch, json!({ "THB": 100000, "USD": 1000 }))
            .await
    }

    async fn open_shift_id(&self, branch: &str) -> String {
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch)
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("an open shift")
    }

    /// Who opened , as stored.
    async fn opened_by(&self, shift: &str) -> String {
        sqlx::query_scalar("SELECT opened_by FROM shifts WHERE id = $1")
            .bind(shift)
            .fetch_one(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("shifts.opened_by of {shift}: {e}"))
    }

    /// The open shift's expected figures as the server keeps them, `code → amount`.
    async fn expected_of_open_shift(&self, branch: &str) -> BTreeMap<String, String> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT b.code::text, b.expected::text FROM shift_balances b \
             JOIN shifts s ON s.id = b.shift_id WHERE s.branch_id = $1 AND s.status = 'OPEN'",
        )
        .bind(branch)
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read the open shift's balances");
        rows.into_iter()
            .map(|(c, e)| (c.trim().to_owned(), e))
            .collect()
    }

    /// The owner closes the open shift of `branch`, counting `counted` (a JSON object), against
    /// what the server expects.
    async fn close_shift_counting(&self, branch: &str, counted: Value, note: &str) -> String {
        let shift = self.open_shift_id(branch).await;
        let expected = self.expected_of_open_shift(branch).await;
        self.ok(
            "OWNER",
            "adminCloseShift",
            json!({ "branchId": branch, "requestId": request_id(), "shiftId": shift,
                    "expected": expected, "counted": counted, "note": note }),
        )
        .await;
        shift
    }

    /// The owner closes the open shift of `branch`, counting exactly what is expected.
    async fn close_shift(&self, branch: &str) -> String {
        let expected = self.expected_of_open_shift(branch).await;
        self.close_shift_counting(branch, json!(expected), "นับเงินปิดกะ")
            .await
    }

    /// The owner moves stock in the open shift of `branch`; the new movement's id.
    async fn move_stock(
        &self,
        branch: &str,
        currency: &str,
        direction: &str,
        amount: &str,
        reason: &str,
    ) -> String {
        let shift = self.open_shift_id(branch).await;
        let req = request_id();
        self.ok(
            "OWNER",
            "adminMoveStock",
            json!({ "branchId": branch, "requestId": req, "shiftId": shift,
                    "currency": currency, "direction": direction, "amount": amount,
                    "reason": reason }),
        )
        .await;
        sqlx::query_scalar(
            "SELECT id FROM stock_movements WHERE branch_id = $1 AND request_id = $2",
        )
        .bind(branch)
        .bind(&req)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("the movement just written")
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
    /// of its side, in the open shift of `branch`. Its document ref.
    async fn save(&self, branch: &str, ex: Ex) -> String {
        let t = self.tick();
        let here = BranchId::new(branch);
        let person = ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: "LIVE".to_owned(),
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
        let cash = ex.method == "Cash";
        let slip = if cash {
            None
        } else {
            Some(
                self.desk
                    .files
                    .upload(&here, "owner", "image/jpeg", JPEG, t)
                    .await
                    .expect("upload a slip"),
            )
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
            bank_fee: "0".to_owned(),
            method: ex.method.to_owned(),
            payment_ref: if cash {
                String::new()
            } else {
                format!("TRF-{:08}", next())
            },
            account: if cash {
                "Cash drawer 1".to_owned()
            } else {
                "Bank account 1".to_owned()
            },
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: "2026-10-06".to_owned(),
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

// ── who may read it, who may write ───────────────────────────────────────────

/// Ticket §8: the three views answer whoever `adminSnapshot` admits on `Rpc` (W8, §4.4), and the
/// writes keep v2's rows — open / close BACKEND and MANAGER, move FINANCE (`policy.rs:83-86`): a
/// FRONT caller's open and close and a BACKEND caller's move get the policy's refusal, unchanged.
#[actix_web::test]
async fn the_cash_views_answer_exactly_whom_the_snapshot_row_allows_and_the_writes_keep_their_rows()
{
    let room = Room::open().await;
    for view in VIEW_NAMES {
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

    // the writes: the role rows of v2's actions, unchanged (W8)
    for (who, action) in [
        ("FRONT", "adminOpenShift"),
        ("FRONT", "adminCloseShift"),
        ("BACKEND", "adminMoveStock"),
    ] {
        let actor = Actor::staff(
            "employee",
            Role::parse(who).expect("a role"),
            BranchId::new(MAIN),
        );
        let refusal = Policy::allow(actor.as_ref(), EntryPath::Rpc, &Action::new(action))
            .expect_err("the policy refuses this pair");
        let answer = room
            .post(
                who,
                action,
                json!({ "branchId": MAIN, "requestId": request_id(),
                        "opening": { "THB": 1 } }),
            )
            .await;
        assert_eq!(
            answer.status,
            refusal.status(),
            "{action} as {who}: {:.300}",
            answer.text
        );
        assert!(
            answer.text.contains(&refusal.to_string()),
            "{action} as {who}: the policy's sentence: {:.300}",
            answer.text
        );
    }
    let data = room.cash("branch=MAIN").await;
    assert_eq!(data["shift"], Value::Null, "nothing was opened: {data}");
    room.finish().await;
}

// ── the answer's shape with no shift ─────────────────────────────────────────

#[actix_web::test]
async fn no_open_shift_answers_no_stock_no_stats_an_empty_reference_and_the_register() {
    let room = Room::open().await;
    let data = room.cash("branch=MAIN").await;
    assert_eq!(keys(&data), set_of(&DATA_FIELDS), "{data}");
    assert_eq!(data["shift"], Value::Null, "{data}");
    assert_eq!(
        data["stock"],
        json!([]),
        "no shift: the empty state of :105: {data}"
    );
    assert_eq!(
        data["stats"],
        Value::Null,
        "no shift: '—' in every card (:102): {data}"
    );
    assert_eq!(
        keys(&data["reference"]),
        set_of(&REFERENCE_FIELDS),
        "{data}"
    );
    assert_eq!(
        data["reference"],
        json!({ "withReference": 0, "withoutReference": 0, "spread": "0.00", "percent": "0.00" }),
        "{data}"
    );
    assert_eq!(
        data["lastCounted"],
        Value::Null,
        "no shift ever closed: {data}"
    );
    let currencies: Vec<String> = list(&data["currencies"])
        .iter()
        .map(|c| c.as_str().expect("a code").to_string())
        .collect();
    assert_eq!(currencies.len(), 153, "the whole register: {currencies:?}");
    assert_eq!(
        currencies,
        register(),
        "THB first, then A–Z (admin-html:26)"
    );

    // `all` changes nothing while no shift is open
    let data = room.cash("branch=MAIN&all=true").await;
    assert_eq!(data["stock"], json!([]), "{data}");
    room.finish().await;
}

// ── the open shift, its stock and its cash flow ──────────────────────────────

/// `admin-html:101-105`, `:37`: the banner's shift, the stock per shown currency
/// (`{opening, net, expected}`, `net = expected − opening`) and v2's cash flow — THB moves only on
/// a cash exchange (`:32`) and on a THB movement; a bank fee never; the remaining is the
/// opening plus what came in less what went out.
#[actix_web::test]
async fn the_open_shift_its_stock_and_its_cash_flow_are_the_servers_figures() {
    let room = Room::open().await;
    let opened_at = room.tick();
    let answer = room
        .post_now(
            "OWNER",
            "adminOpenShift",
            json!({ "branchId": MAIN, "requestId": request_id(),
                    "opening": { "THB": "100000", "USD": "1000" } }),
        )
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let shift = room.open_shift_id(MAIN).await;

    room.save(MAIN, Ex::buy("100", "50")).await; // USD +100, THB −2950 (paid out)
    room.save(MAIN, Ex::sell("10", "20")).await; // USD −10, THB +420 (received)
    room.save(MAIN, Ex::buy("1", "0").transfer()).await; // USD +1, THB untouched
    room.move_stock(MAIN, "THB", "IN", "500", "เงินเติมลิ้นชัก")
        .await; // THB +500
    room.move_stock(MAIN, "THB", "OUT", "200", "นำเงินฝากธนาคาร")
        .await; // THB −200
    room.move_stock(MAIN, "USD", "IN", "9.5", "รับจากเซฟ").await; // USD +9.50

    let data = room.cash("branch=MAIN").await;
    assert_eq!(keys(&data["shift"]), set_of(&SHIFT_FIELDS), "{data}");
    assert_eq!(data["shift"]["id"], json!(shift), "{data}");
    assert_eq!(data["shift"]["openedAt"], json!(iso(opened_at)), "{data}");
    assert_eq!(
        data["shift"]["staff"],
        json!(room.opened_by(&shift).await),
        "the one who opened it, as stored: {data}"
    );

    let stock = stock_of(&data);
    assert_eq!(
        stock,
        vec![
            (
                "THB".to_string(),
                "100000.00".to_string(),
                "-2230.00".to_string(),
                "97770.00".to_string()
            ),
            (
                "USD".to_string(),
                "1000.00".to_string(),
                "100.50".to_string(),
                "1100.50".to_string()
            ),
        ],
        "THB, then the rated USD; two-place strings; net = expected − opening: {data}"
    );
    let kept = room.expected_of_open_shift(MAIN).await;
    for (code, _, _, expected) in &stock {
        assert_eq!(
            dec(expected),
            dec(&kept[code]),
            "{code}: expected is the server's running figure"
        );
    }

    assert_eq!(keys(&data["stats"]), set_of(&STATS_FIELDS), "{data}");
    assert_eq!(
        data["stats"],
        json!({ "opening": "100000.00", "incoming": "920.00",
                "outgoing": "3150.00", "remaining": "97770.00" }),
        "v2's cashFlow (admin-html:37): {data}"
    );
    room.finish().await;
}

/// `admin-workspace.js:125-131`: shown are THB, every rated currency (even at zero), and every
/// currency with a non-zero opening, net or expected; `all=true` answers the whole register.
#[actix_web::test]
async fn the_stock_shows_thb_the_rated_and_the_non_zero_and_every_currency_on_request() {
    let room = Room::open().await;
    room.set_rate(MAIN, "EUR", "35", "38").await; // rated, nothing in the drawer
    room.open_shift_with(
        MAIN,
        json!({ "THB": "50000", "USD": "10", "CHF": "10", "JPY": "2500" }),
    )
    .await;
    // CHF goes to zero: opening 10, net −10, expected 0 — still shown
    room.move_stock(MAIN, "CHF", "OUT", "10", "ส่งคืนเซฟ").await;
    // AUD arrives during the shift: opening 0, net +5
    room.move_stock(MAIN, "AUD", "IN", "5", "รับจากเซฟ").await;

    let data = room.cash("branch=MAIN").await;
    let stock = stock_of(&data);
    assert_eq!(
        codes_of(&data),
        vec!["THB", "AUD", "CHF", "EUR", "JPY", "USD"],
        "THB first, then A–Z; GBP and the rest of the register hidden: {data}"
    );
    let row = |code: &str| {
        stock
            .iter()
            .find(|r| r.0 == code)
            .cloned()
            .unwrap_or_else(|| panic!("{code} in {stock:?}"))
    };
    assert_eq!(
        row("CHF"),
        ("CHF".into(), "10.00".into(), "-10.00".into(), "0.00".into())
    );
    assert_eq!(
        row("AUD"),
        ("AUD".into(), "0.00".into(), "5.00".into(), "5.00".into())
    );
    assert_eq!(
        row("EUR"),
        ("EUR".into(), "0.00".into(), "0.00".into(), "0.00".into()),
        "a rated currency at zero is shown"
    );

    let all = room.cash("branch=MAIN&all=true").await;
    assert_eq!(
        codes_of(&all),
        register(),
        "all=true: every currency of the register, THB first then A–Z"
    );
    for (code, opening, net, expected) in stock_of(&all) {
        if !["THB", "AUD", "CHF", "EUR", "JPY", "USD"].contains(&code.as_str()) {
            assert_eq!(
                (opening.as_str(), net.as_str(), expected.as_str()),
                ("0.00", "0.00", "0.00"),
                "{code}: a currency nobody holds"
            );
        }
    }
    assert_eq!(
        all["stats"], data["stats"],
        "the filter changes the rows only"
    );

    let explicit = room.cash("branch=MAIN&all=false").await;
    assert_eq!(
        codes_of(&explicit),
        codes_of(&data),
        "all=false is the default"
    );

    for bad in ["yes", "1x", "TRUE!"] {
        let answer = room
            .get("OWNER", &view_path(CASH, &format!("branch=MAIN&all={bad}")))
            .await;
        assert_eq!(answer.status, 400, "all={bad}: {:.300}", answer.text);
        let body = answer.json();
        assert_eq!(
            body["error"]["code"],
            json!("invalidRequest"),
            "all={bad}: {body}"
        );
        assert_eq!(body["error"]["field"], json!("all"), "all={bad}: {body}");
    }
    room.finish().await;
}

// ── the reference summary ────────────────────────────────────────────────────

/// `admin-html:144` over the open shift's exchanges (`:103`): how many carry a reference rate and
/// how many do not; the spread (BUY: reference − actual, SELL: actual − reference, each at two
/// places) and its share of the reference value, two places. A closed shift's exchanges are not
/// the open shift's.
#[actix_web::test]
async fn the_reference_summary_is_v2s_over_the_open_shifts_exchanges() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;
    room.save(MAIN, Ex::buy("1", "0").reference("31")).await; // an earlier shift's
    room.close_shift(MAIN).await;

    room.open_shift(MAIN).await;
    let data = room.cash("branch=MAIN").await;
    assert_eq!(
        data["reference"],
        json!({ "withReference": 0, "withoutReference": 0, "spread": "0.00", "percent": "0.00" }),
        "the new shift has no exchange: {data}"
    );

    // BUY 100 at 30, reference 32: reference 3200, actual 3000, spread +200
    room.save(MAIN, Ex::buy("100", "0").reference("32")).await;
    // SELL 10 at 40, reference 36: reference 360, actual 400, spread +40
    room.save(MAIN, Ex::sell("10", "0").reference("36")).await;
    // no reference rate
    room.save(MAIN, Ex::buy("5", "0")).await;
    let data = room.cash("branch=MAIN").await;
    assert_eq!(
        data["reference"],
        json!({ "withReference": 2, "withoutReference": 1,
                "spread": "240.00", "percent": "6.74" }),
        "240 / 3560 × 100 = 6.7415… → 6.74: {data}"
    );
    room.finish().await;
}

// ── the opening prefill ──────────────────────────────────────────────────────

/// `admin-opening-currencies.js:9`, `:18-19` and `admin-html:127`: the opening dialog starts from
/// the last closed shift's count — THB always, then each other currency counted above zero, in
/// register order; nothing before any shift closed.
#[actix_web::test]
async fn last_counted_is_the_last_closed_shifts_count_thb_always_then_the_non_zero() {
    let room = Room::open().await;
    assert_eq!(room.cash("branch=MAIN").await["lastCounted"], Value::Null);

    room.open_shift_with(MAIN, json!({ "THB": "1000", "USD": "50", "EUR": "20" }))
        .await;
    assert_eq!(
        room.cash("branch=MAIN").await["lastCounted"],
        Value::Null,
        "an open shift is not a closed one"
    );
    room.close_shift_counting(
        MAIN,
        json!({ "THB": "0", "USD": "45", "EUR": "20", "AUD": "0" }),
        "THB ฝากธนาคาร · USD ขาด 5",
    )
    .await;
    let data = room.cash("branch=MAIN").await;
    assert_eq!(
        data["lastCounted"],
        json!([ { "code": "THB", "amount": "0.00" },
                { "code": "EUR", "amount": "20.00" },
                { "code": "USD", "amount": "45.00" } ]),
        "THB even at zero, then A–Z: {data}"
    );

    room.open_shift_with(MAIN, json!({ "THB": "700", "JPY": "300" }))
        .await;
    room.close_shift(MAIN).await;
    assert_eq!(
        room.cash("branch=MAIN").await["lastCounted"],
        json!([ { "code": "THB", "amount": "700.00" },
                { "code": "JPY", "amount": "300.00" } ]),
        "the newest closed shift's count"
    );
    assert_eq!(
        room.cash("branch=B9").await["lastCounted"],
        Value::Null,
        "another branch's shifts are not B9's"
    );
    room.finish().await;
}

// ── two open shifts ──────────────────────────────────────────────────────────

/// Ticket §9 row 1 (v2 `admin-html:31`): v3 cannot hold two OPEN shifts of one branch — the
/// database refuses the second, and a second `adminOpenShift` gets the server's sentence; the view
/// keeps answering the one that is open.
#[actix_web::test]
async fn a_second_open_shift_is_refused_with_the_servers_sentence() {
    let room = Room::open().await;
    let first = room.open_shift(MAIN).await;
    let answer = room
        .post(
            "OWNER",
            "adminOpenShift",
            json!({ "branchId": MAIN, "requestId": request_id(), "opening": { "THB": 1 } }),
        )
        .await;
    assert_eq!(answer.status, 400, "{:.300}", answer.text);
    assert!(
        answer.text.contains("มีกะเปิดอยู่แล้ว กรุณารีเฟรช"),
        "the server's sentence: {:.300}",
        answer.text
    );
    let inserted = sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id) \
         VALUES ('S-TWIN', 'MAIN', 'OPEN', now(), 'test', 'twin-0000000000000001')",
    )
    .execute(&room.desk.db.pool)
    .await;
    assert!(
        inserted.is_err(),
        "the database holds one OPEN shift per branch"
    );
    let data = room.cash("branch=MAIN").await;
    assert_eq!(data["shift"]["id"], json!(first), "{data}");
    room.finish().await;
}

// ── the movements page ───────────────────────────────────────────────────────

/// `admin-html:106`: the branch's movements, newest recorded first — by the order they were
/// written, not by their time — across shifts, each with v2's fields; keyset paging by `before`.
#[actix_web::test]
async fn the_movements_page_is_newest_recorded_first_across_shifts_with_keyset_paging() {
    let room = Room::open().await;
    let first_shift = room.open_shift(MAIN).await;
    let m1 = room
        .move_stock(MAIN, "THB", "IN", "500", "เงินเติมลิ้นชัก")
        .await;
    let m2 = room.move_stock(MAIN, "USD", "OUT", "10.25", "ส่งเซฟ").await;
    room.close_shift(MAIN).await;
    let second_shift = room.open_shift(MAIN).await;
    let m3 = room.move_stock(MAIN, "THB", "OUT", "200", "ค่าใช้จ่าย").await;
    // written last, with a clock set before the others: still the newest row
    room.desk.clock.set(room.t0 - Duration::days(3));
    let req = request_id();
    let answer = room
        .post_now(
            "OWNER",
            "adminMoveStock",
            json!({ "branchId": MAIN, "requestId": req, "shiftId": second_shift,
                    "currency": "USD", "direction": "IN", "amount": "1", "reason": "ย้อนเวลา" }),
        )
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let m4: String = sqlx::query_scalar(
        "SELECT id FROM stock_movements WHERE branch_id = $1 AND request_id = $2",
    )
    .bind(MAIN)
    .bind(&req)
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("the last movement");
    // another branch's movement is never MAIN's
    room.open_shift(B9).await;
    let other = room.move_stock(B9, "THB", "IN", "1", "สาขาอื่น").await;

    let rows = room.data(MOVEMENTS, "OWNER", "branch=MAIN").await;
    assert_eq!(
        ids(&rows),
        vec![m4.clone(), m3.clone(), m2.clone(), m1.clone()],
        "newest recorded first: {rows}"
    );
    assert!(!ids(&rows).contains(&other), "{rows}");

    let row = list(&rows)
        .iter()
        .find(|r| r["id"] == json!(m2))
        .expect("m2")
        .clone();
    assert_eq!(keys(&row), set_of(&MOVEMENT_FIELDS), "{row}");
    assert_eq!(row["shiftId"], json!(first_shift), "{row}");
    assert_eq!(row["direction"], json!("OUT"), "{row}");
    assert_eq!(row["currency"], json!("USD"), "{row}");
    assert_eq!(row["amount"], json!("10.25"), "two places, a string: {row}");
    assert_eq!(row["reason"], json!("ส่งเซฟ"), "{row}");
    let by: String = sqlx::query_scalar("SELECT by FROM stock_movements WHERE id = $1")
        .bind(&m2)
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("who moved it");
    assert_eq!(row["staff"], json!(by), "{row}");
    let at: DateTime<Utc> = sqlx::query_scalar("SELECT at FROM stock_movements WHERE id = $1")
        .bind(&m2)
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("when");
    assert_eq!(row["created"], json!(iso(at)), "{row}");
    let newest = &list(&rows)[0];
    assert_eq!(newest["shiftId"], json!(second_shift), "{newest}");
    assert_eq!(newest["direction"], json!("IN"), "{newest}");

    // keyset paging
    let page1 = room.data(MOVEMENTS, "OWNER", "branch=MAIN&limit=2").await;
    assert_eq!(ids(&page1), vec![m4.clone(), m3.clone()], "{page1}");
    let page2 = room
        .data(
            MOVEMENTS,
            "OWNER",
            &format!("branch=MAIN&limit=2&before={m3}"),
        )
        .await;
    assert_eq!(ids(&page2), vec![m2.clone(), m1.clone()], "{page2}");
    let page3 = room
        .data(
            MOVEMENTS,
            "OWNER",
            &format!("branch=MAIN&limit=2&before={m1}"),
        )
        .await;
    assert_eq!(page3, json!([]), "{page3}");
    room.finish().await;
}

// ── the shifts page ──────────────────────────────────────────────────────────

/// `admin-html:107`, `:130`: the branch's shifts, newest opened first, each with its status, who
/// opened it, its note, the non-zero differences of its count and the detail rows; keyset paging.
#[actix_web::test]
async fn the_shifts_page_is_newest_first_with_v2s_difference_and_the_detail_rows() {
    let room = Room::open().await;
    // 1: closed short — USD counted 5 under, EUR counted 2 over, THB as expected
    let s1 = room
        .open_shift_with(MAIN, json!({ "THB": "1000", "USD": "100", "EUR": "10" }))
        .await;
    room.close_shift_counting(
        MAIN,
        json!({ "THB": "1000", "USD": "95", "EUR": "12" }),
        "USD ขาด 5 · EUR เกิน 2",
    )
    .await;
    // 2: closed exactly
    let s2 = room.open_shift_with(MAIN, json!({ "THB": "2000" })).await;
    room.close_shift(MAIN).await;
    // 3: open, with a movement
    let s3 = room
        .open_shift_with(MAIN, json!({ "THB": "3000", "USD": "1" }))
        .await;
    room.move_stock(MAIN, "JPY", "IN", "700", "รับจากเซฟ").await;
    room.open_shift(B9).await;

    let rows = room.data(SHIFTS, "OWNER", "branch=MAIN").await;
    assert_eq!(
        ids(&rows),
        vec![s3.clone(), s2.clone(), s1.clone()],
        "newest first: {rows}"
    );
    for row in list(&rows) {
        assert_eq!(keys(row), set_of(&SHIFT_ROW_FIELDS), "{row}");
        let id = row["id"].as_str().expect("an id");
        assert_eq!(row["staff"], json!(room.opened_by(id).await), "{row}");
        let opened: DateTime<Utc> =
            sqlx::query_scalar("SELECT opened_at FROM shifts WHERE id = $1")
                .bind(id)
                .fetch_one(&room.desk.db.pool)
                .await
                .expect("opened_at");
        assert_eq!(row["openedAt"], json!(iso(opened)), "{row}");
        for balance in list(&row["balances"]) {
            assert_eq!(keys(balance), set_of(&BALANCE_FIELDS), "{balance}");
        }
    }
    let by_id = |id: &str| -> Value {
        list(&rows)
            .iter()
            .find(|r| r["id"] == json!(id))
            .cloned()
            .unwrap_or_else(|| panic!("{id} in {rows}"))
    };

    let one = by_id(&s1);
    assert_eq!(one["status"], json!("CLOSED"), "{one}");
    assert_eq!(one["note"], json!("USD ขาด 5 · EUR เกิน 2"), "{one}");
    assert_eq!(
        one["difference"],
        json!([ { "code": "EUR", "amount": "2.00" }, { "code": "USD", "amount": "-5.00" } ]),
        "the non-zero counted − expected, A–Z after THB (admin-html:107): {one}"
    );
    assert_eq!(
        one["balances"],
        json!([
            { "code": "THB", "expected": "1000.00", "counted": "1000.00", "difference": "0.00" },
            { "code": "EUR", "expected": "10.00", "counted": "12.00", "difference": "2.00" },
            { "code": "USD", "expected": "100.00", "counted": "95.00", "difference": "-5.00" }
        ]),
        "the detail rows of :130: {one}"
    );

    let two = by_id(&s2);
    assert_eq!(two["status"], json!("CLOSED"), "{two}");
    assert_eq!(
        two["difference"],
        json!([]),
        "counted as expected: ตรงตามระบบ: {two}"
    );
    assert_eq!(
        two["balances"],
        json!([ { "code": "THB", "expected": "2000.00", "counted": "2000.00", "difference": "0.00" } ]),
        "{two}"
    );

    let three = by_id(&s3);
    assert_eq!(three["status"], json!("OPEN"), "{three}");
    assert_eq!(
        three["difference"],
        Value::Null,
        "an open shift has no count: {three}"
    );
    assert_eq!(three["note"], json!(""), "{three}");
    assert_eq!(
        three["balances"],
        json!([
            { "code": "THB", "expected": "3000.00", "counted": null, "difference": null },
            { "code": "JPY", "expected": "700.00", "counted": null, "difference": null },
            { "code": "USD", "expected": "1.00", "counted": null, "difference": null }
        ]),
        "an open shift's running expected (admin-html:130 balanceOf): {three}"
    );

    let page1 = room.data(SHIFTS, "OWNER", "branch=MAIN&limit=1").await;
    assert_eq!(ids(&page1), vec![s3.clone()], "{page1}");
    let page2 = room
        .data(SHIFTS, "OWNER", &format!("branch=MAIN&limit=2&before={s3}"))
        .await;
    assert_eq!(ids(&page2), vec![s2.clone(), s1.clone()], "{page2}");
    let page3 = room
        .data(SHIFTS, "OWNER", &format!("branch=MAIN&before={s1}"))
        .await;
    assert_eq!(page3, json!([]), "{page3}");
    room.finish().await;
}

// ── branches ─────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_owner_reads_the_branch_named_and_staff_their_own() {
    let room = Room::open().await;
    let nine = room.open_shift(B9).await;
    let moved = room.move_stock(B9, "THB", "IN", "1", "สาขา B9").await;

    let cash = room.data(CASH, "OWNER", "branch=B9").await;
    assert_eq!(cash["shift"]["id"], json!(nine), "{cash}");
    assert_eq!(
        ids(&room.data(MOVEMENTS, "OWNER", "branch=B9").await),
        vec![moved.clone()]
    );
    assert_eq!(
        ids(&room.data(SHIFTS, "OWNER", "branch=B9").await),
        vec![nine.clone()]
    );

    for (who, query) in [
        ("OWNER", "branch=MAIN"),
        ("OWNER", ""),
        ("FRONT", "branch=B9"),
        ("FINANCE", "branch=B9"),
    ] {
        let cash = room.data(CASH, who, query).await;
        assert_eq!(
            cash["shift"],
            Value::Null,
            "{who} {query:?}: MAIN's: {cash}"
        );
        assert_eq!(cash["stock"], json!([]), "{who} {query:?}: {cash}");
        assert_eq!(
            room.data(MOVEMENTS, who, query).await,
            json!([]),
            "{who} {query:?}"
        );
        assert_eq!(
            room.data(SHIFTS, who, query).await,
            json!([]),
            "{who} {query:?}"
        );
    }
    room.finish().await;
}

// ── §4.2: every writer moves the version ─────────────────────────────────────

/// One write per writer of what the three views read, each through `BranchTx`: the same `since`
/// then answers `changed: true` with the write in it; before it, an equal `since` is exactly
/// unchanged. The two pages are read again after each write and show it.
#[actix_web::test]
async fn every_writer_of_the_cash_views_moves_the_version_they_answer() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    assert_eq!(
        room.view(CASH, "OWNER", &format!("branch=MAIN&since={v}"))
            .await,
        json!({ "version": v, "changed": false }),
        "an equal since: exactly the version and changed: false"
    );

    let mut since = v;
    for writer in [
        "adminOpenShift",
        "save",
        "adminMoveStock",
        "adminAbandonRequest",
        "adminCloseShift",
    ] {
        let before = room.cash("branch=MAIN").await;
        match writer {
            "adminOpenShift" => {
                room.open_shift(MAIN).await;
            }
            "save" => {
                room.save(MAIN, Ex::buy("10", "0")).await;
            }
            "adminMoveStock" => {
                room.move_stock(MAIN, "THB", "IN", "10", "เงินเติม").await;
            }
            "adminAbandonRequest" => room.abandon(MAIN).await,
            "adminCloseShift" => {
                room.close_shift(MAIN).await;
            }
            _ => unreachable!(),
        }
        let now = room.version(MAIN).await;
        assert!(
            now > since,
            "{writer} moved MAIN's version ({since} → {now})"
        );
        let body = room
            .view(CASH, "OWNER", &format!("branch=MAIN&since={since}"))
            .await;
        assert_eq!(body["version"], json!(now), "after {writer}: {body}");
        assert_eq!(body["changed"], json!(true), "after {writer}: {body}");
        let after = body["data"].clone();
        if writer != "adminAbandonRequest" {
            assert_ne!(after, before, "{writer} shows in admin_cash");
        }
        assert_eq!(
            room.view(CASH, "OWNER", &format!("branch=MAIN&since={now}"))
                .await,
            json!({ "version": now, "changed": false }),
            "after {writer}, the new version is unchanged"
        );
        since = now;
    }
    let shifts = room.data(SHIFTS, "OWNER", "branch=MAIN").await;
    assert_eq!(list(&shifts).len(), 1, "{shifts}");
    assert_eq!(list(&shifts)[0]["status"], json!("CLOSED"), "{shifts}");
    assert_eq!(
        list(&room.data(MOVEMENTS, "OWNER", "branch=MAIN").await).len(),
        1
    );
    room.finish().await;
}

/// Ticket §7: custody's shift effects (`custody/service.rs` — open, close and move on the staff
/// route) are writers too. The recorded custody day is run through HTTP; after each step, when any
/// of the three views answers different data than before the step, the branch's version has moved
/// and `admin_cash` with the old version as `since` answers `changed: true`. The day must open a
/// shift, close one and move stock through custody, or the test proves nothing.
#[actix_web::test]
async fn custodys_shift_effects_move_the_version_the_cash_views_answer() {
    let fixture = Fixture::load(OPS_FILE);
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    for n in OPS_UNKNOWN_NAME_BY_STATE {
        replay = replay.rule(
            n,
            Rule::Differs {
                status: 400,
                sentence: INVALID_REQUEST,
            },
        );
    }
    for n in OPS_OWNERS_DOOR {
        replay = replay.rule(
            n,
            Rule::Excluded("`/api/employees`: the owner's door to custody is another ticket's"),
        );
    }

    async fn read(app: &App, cookie: &str, view: &str) -> (i64, Value) {
        let answer = app
            .send(get(&view_path(view, "branch=MAIN")).cookie(cookie))
            .await;
        assert_eq!(answer.status, 200, "{view}: {:.300}", answer.text);
        let body = answer.json();
        (
            body["version"].as_i64().expect("a version"),
            body["data"].clone(),
        )
    }

    let owner = replay.cookies["owner"].clone();
    let mut opened = 0;
    let mut closed = 0;
    let mut moved = 0;
    let mut changes = 0;
    for n in 1..=OPS_LAST {
        let step = fixture.step(n);
        if n == OPS_SWITCHED_ON_AT {
            replay.desk.custody_on(MAIN).await;
        }
        let (v_before, cash_before) = read(&replay.app, &owner, CASH).await;
        let (_, moves_before) = read(&replay.app, &owner, MOVEMENTS).await;
        let (_, shifts_before) = read(&replay.app, &owner, SHIFTS).await;
        replay.step(step).await;
        if n == OPS_HOLD_MADE_AT {
            let answer = replay
                .app
                .send(
                    Call::action(ORIGIN, RPC, "adminSnapshot", json!({ "branchId": MAIN }))
                        .cookie(&owner),
                )
                .await;
            let snapshot = answer.json();
            let holds = snapshot["operations"]["holds"]
                .as_array()
                .expect("the holds of the branch");
            let recorded = &fixture
                .step(OPS_HOLD_NAMED_AT)
                .payload
                .as_ref()
                .expect("a payload")["id"];
            replay.ids.learn(recorded, &holds[0]["id"]);
        }
        let (v_after, cash_after) = read(&replay.app, &owner, CASH).await;
        let (_, moves_after) = read(&replay.app, &owner, MOVEMENTS).await;
        let (_, shifts_after) = read(&replay.app, &owner, SHIFTS).await;
        if cash_after != cash_before || moves_after != moves_before || shifts_after != shifts_before
        {
            changes += 1;
            assert!(
                v_after > v_before,
                "ops_actions#{n} ({:?}) changed the cash views without moving the version",
                step.action
            );
            let answer = replay
                .app
                .send(
                    get(&view_path(CASH, &format!("branch=MAIN&since={v_before}"))).cookie(&owner),
                )
                .await;
            assert_eq!(answer.json()["changed"], json!(true), "ops_actions#{n}");
            if cash_before["shift"].is_null() && !cash_after["shift"].is_null() {
                opened += 1;
            }
            if !cash_before["shift"].is_null() && cash_after["shift"] != cash_before["shift"] {
                closed += 1;
            }
            if list(&moves_after).len() > list(&moves_before).len() {
                moved += 1;
            }
        }
    }
    assert!(changes > 0, "the custody day changed the cash views");
    assert!(opened > 0, "custody opened a shift ({changes} changes)");
    assert!(closed > 0, "custody closed a shift ({changes} changes)");
    assert!(moved > 0, "custody moved stock ({changes} changes)");
    replay.finish().await;
}

// ── the generated types ──────────────────────────────────────────────────────

fn exported(view: &str) -> BTreeMap<String, String> {
    let entry = VIEWS
        .iter()
        .find(|v| v.name == view)
        .unwrap_or_else(|| panic!("the view {view} is in the generated table"));
    let dir: PathBuf =
        std::env::temp_dir().join(format!("xc-467-{}-{}", std::process::id(), next()));
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
fn the_three_cash_views_are_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    for view in VIEW_NAMES {
        assert!(names.contains(&view), "{view} in {names:?}");
    }
}

#[test]
fn each_cash_view_writes_its_own_file_money_as_strings() {
    for view in VIEW_NAMES {
        let files = exported(view);
        let own = format!("{view}.ts");
        for name in files.keys() {
            assert!(
                name == &own || name == "common.ts",
                "{view} writes only {own} and common.ts, not {name}"
            );
        }
        let ts = files
            .get(&own)
            .unwrap_or_else(|| panic!("{own} in {:?}", files.keys()));
        assert!(!ts.contains("bigint"), "no bigint on the wire:\n{ts}");
        assert!(
            !ts.contains(": number,") || view == CASH,
            "{view}: no number field:\n{ts}"
        );
    }

    let cash = exported(CASH);
    let ts = &cash["admin_cash.ts"];
    for field in [
        "opening",
        "net",
        "expected",
        "incoming",
        "outgoing",
        "remaining",
        "spread",
        "percent",
        "amount",
    ] {
        has_field(ts, field, &["string"]);
    }
    has_field(ts, "withReference", &["number"]);
    has_field(ts, "withoutReference", &["number"]);
    has_field(ts, "all", &["boolean"]);
    has_field(ts, "currencies", &["Array<string>", "string[]"]);

    let moves = exported(MOVEMENTS);
    let ts = &moves["admin_cash_movements.ts"];
    for field in MOVEMENT_FIELDS {
        has_field(ts, field, &["string"]);
    }

    let shifts = exported(SHIFTS);
    let ts = &shifts["admin_shifts.ts"];
    for field in ["id", "status", "openedAt", "staff", "note", "expected"] {
        has_field(ts, field, &["string"]);
    }
    has_field(ts, "counted", &["string | null"]);
}

#[actix_web::test]
async fn the_reference_summary_rounds_midpoint_half_up() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;
    // BUY 1 at 30, reference 30.525: ref_value = 30.525 -> 30.53 (banker's would be 30.52).
    // actual = 30.00, spread = 30.53 - 30.00 = +0.53.
    // percent = 0.53 / 30.53 * 100 = 1.7359... -> 1.74.
    room.save(MAIN, Ex::buy("1", "0").reference("30.525")).await;
    let data = room.cash("branch=MAIN").await;
    assert_eq!(
        data["reference"],
        json!({
            "withReference": 1,
            "withoutReference": 0,
            "spread": "0.53",
            "percent": "1.74"
        }),
        "half-up midpoint rounding: {data}"
    );
    room.finish().await;
}
