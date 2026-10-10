//! Ticket #400 (V1-devices-views, `docs/design/xchange-v3-web.md` §4, §5.1, §6 row
//! `/devices`): the four bounded views the v3 devices screen reads instead of the whole-branch
//! `snapshot` of `POST /api/devices`. Written before the implementation by the lane
//! controller. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/devices` — a `BranchView`: `{branchId, branches, printerOnline,
//!   printerKind, printerModel, hasDrawer, drawerAgentActive, balanceThb, balanceUsd,
//!   completedCount}` (ticket §7.1).
//! - `GET /api/v3/views/device_jobs`, `device_events`, `device_transactions` — `PagedView`s,
//!   newest first, `before` = the row id, `limit` ≤ 100, 50 when absent (§7.2–§7.4).
//! - All four inherit `(EntryPath::Devices, "snapshot")`: they answer exactly whom
//!   `Policy::allow` lets read the devices snapshot, on the branch `Policy::branch` gives.
//! - Money is a decimal string computed by the server (W7): the balance is the server's
//!   figure (§7.5), never a sum the browser could make.
//!
//! **Which figure the balance metric shows (§7.5).** v2's page recomputed the balance in the
//! browser (`xchange-v2-source@4d0bdbb:src/devices-public/devices.js:5`): the open shift's
//! opening, plus its completed exchanges (baht only for cash), plus its stock movements; the
//! newest shift's `counted` when none is open. v3's server already keeps that sum as the open
//! shift's `expected` (`xc-shift` `StockDelta::for_tx`, `apply_delta_in`). Where the two could
//! differ — anything that changes `expected` without being an exchange or a movement of the
//! shift — **the metric is the server's stored `expected`**, never a recomputation. The test
//! `the_balance_is_the_shifts_stored_figure_never_a_recomputation` pins that by changing the
//! stored figure behind the routes.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`), the test's clock advanced between
//! writes (rows are never ordered by a timestamp under a frozen clock). Exchanges are saved
//! through the real `SaveTransactionWorkflow` behind the routes, as `rpc_common` does.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Amended by the ticket author of #603 (RE-6, claude/e5ab6c), item 2: `configure` switches a
//! currency on only when the branch's current publication has it on both sides, so the won is
//! published (behind the routes) before the kiosk `configure` writer runs.

mod common;
mod compat_common;
mod publication_seed;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
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
const DEVICES: &str = "/api/devices";
const SETUP: &str = "/api/device-setup";
const PAIR: &str = "/api/device-agent/pair";
const CLAIM: &str = "/api/device-agent/claim";
const REPORT: &str = "/api/device-agent/report";
const KIOSK: &str = "/api/kiosk";
const RPC: &str = "/api/rpc";

const MAIN: &str = "MAIN";
const B9: &str = "B9";
/// The name the migration gives `MAIN`.
const MAIN_NAME: &str = "XC-B";
const B9_NAME: &str = "Nine";

/// The first printer model of the configuration: the model while nobody has set one.
const NEOCAL: &str = "NEOCAL_NTP_8220BT";
const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";

/// The four views of this ticket.
const VIEW_NAMES: [&str; 4] = [
    "devices",
    "device_jobs",
    "device_events",
    "device_transactions",
];

/// The keys of `data` of the `devices` view (§7.1, §8).
const DEVICES_KEYS: [&str; 10] = [
    "branchId",
    "branches",
    "printerOnline",
    "printerKind",
    "printerModel",
    "hasDrawer",
    "drawerAgentActive",
    "balanceThb",
    "balanceUsd",
    "completedCount",
];
const JOB_KEYS: [&str; 6] = ["id", "kind", "ref", "status", "actor", "created"];
const EVENT_KEYS: [&str; 6] = ["seq", "kind", "ref", "at", "actor", "detail"];
const TRANSACTION_KEYS: [&str; 4] = ["id", "type", "amount", "currency"];

/// The first bytes of a JPEG: enough for a passport upload.
const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A number no other call of this test binary has used.
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

fn decimal(text: &Value, what: &str) -> Decimal {
    let s = text
        .as_str()
        .unwrap_or_else(|| panic!("{what}: money is a decimal string (W7), got {text}"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{what}: {s:?} is a decimal: {e}"))
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

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl Room {
    /// `MAIN` (from the migration) and `B9`, the owner and one employee of every staff role
    /// in `MAIN`, all signed in; the USD counter rate 32.1 / 33 in both branches.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, B9_NAME).await;
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
            room.set_rate(branch, "USD", "32.1", "33").await;
        }
        room
    }

    /// The test's clock moves forward one second on every call: no two writes share an
    /// instant.
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

    /// A view as `who`, which must be answered 200 with JSON.
    async fn view(&self, who: &str, path: &str) -> Value {
        let answer = self.get(who, path).await;
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

    /// `data` of a view read with no `since`.
    async fn data(&self, who: &str, path: &str) -> Value {
        let body = self.view(who, path).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {path} with no since: {body}"
        );
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {path}: data in {body}"))
    }

    async fn post(&self, who: &str, path: &str, action: &str, payload: Value) -> Answer {
        self.tick();
        let call = Call::action(ORIGIN, path, action, payload);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    async fn ok(&self, who: &str, path: &str, action: &str, payload: Value) -> Value {
        let answer = self.post(who, path, action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "{action} on {path} as {who} with {payload}: {:.400}",
            answer.text
        );
        if answer.text.is_empty() {
            Value::Null
        } else {
            answer.json()
        }
    }

    async fn devices(&self, who: &str, action: &str, payload: Value) -> Value {
        self.ok(who, DEVICES, action, payload).await
    }

    /// The owner queues a drawer request in `branch`; the job's id.
    async fn queue_drawer(&self, who: &str, branch: &str) -> String {
        let job = self
            .devices(
                who,
                "queue",
                json!({ "branchId": branch, "kind": "DRAWER_REQUEST", "ref": "",
                        "requestId": request_id(), "reason": "เติมเงินทอน" }),
            )
            .await;
        job["id"].as_str().expect("a job id").to_owned()
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
        // #759 (RE-4e; rates.md D-3, §4.9 R4): the counter rate is the current publication.
        rates_re4e_publish::publish_newest_rates(&self.desk.db.pool, branch).await;
    }

    /// The owner opens a shift in `branch` through `/api/rpc`.
    async fn open_shift(&self, branch: &str, opening: Value) {
        self.ok(
            "OWNER",
            RPC,
            "adminOpenShift",
            json!({ "branchId": branch, "requestId": request_id(), "opening": opening }),
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

    /// The open shift's stored `expected`, `code -> "0.00"`, as a JSON object of strings.
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

    /// The owner closes the open shift of `branch`, counting `counted`.
    async fn close_shift(&self, branch: &str, counted: Value) {
        let shift = self.open_shift_id(branch).await;
        let expected = self.expected_of_open_shift(branch).await;
        self.ok(
            "OWNER",
            RPC,
            "adminCloseShift",
            json!({ "branchId": branch, "requestId": request_id(), "shiftId": shift,
                    "expected": expected, "counted": counted, "note": "นับเงินปิดกะ" }),
        )
        .await;
    }

    /// One exchange of USD saved through the real workflow behind the routes, in `mode`
    /// (`LIVE` needs an open shift), paid in cash; its document number.
    async fn save(&self, branch: &str, side: &str, amount: &str, fee: &str, mode: &str) -> String {
        let t = self.tick();
        let here = BranchId::new(branch);
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
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer,
            side: side.to_owned(),
            currency: "USD".to_owned(),
            amount: amount.to_owned(),
            rate: if side == "BUY" { "32.1" } else { "33" }.to_owned(),
            fee: fee.to_owned(),
            bank_fee: "0".to_owned(),
            method: "Cash".to_owned(),
            payment_ref: String::new(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: "2026-10-06".to_owned(),
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
            .unwrap_or_else(|e| panic!("save a {mode} {side} in {branch}: {e:?}"))
            .document_ref
    }

    /// Every table a view of this ticket can read, renamed away: a view that loads anything
    /// now fails, while `branch_versions` (the `since` check) and the sign-in tables stay.
    /// `branches` stays too: the core's branch check reads it before `since` (#500).
    async fn hide_the_views_tables(&self) {
        for table in [
            "device_jobs",
            "device_events",
            "device_settings",
            "device_agents",
            "shift_balances",
            "stock_movements",
            "shifts",
            "transaction_files",
            "transactions",
            "kiosk_currencies",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE {table} RENAME TO hidden_{table}"
            )))
            .execute(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("rename {table}: {e}"));
        }
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
    }
}

fn is_unchanged(body: &Value, version: i64, what: &str) {
    assert_eq!(
        body,
        &json!({ "version": version, "changed": false }),
        "{what}: exactly the version and changed: false, no data"
    );
}

/// After a write: every view of `MAIN` read with `since = before` reports a change and data.
async fn every_view_changed(room: &Room, before: i64, what: &str) {
    let now = room.version(MAIN).await;
    assert!(
        now > before,
        "{what}: the write moved MAIN's version ({before} → {now})"
    );
    for view in VIEW_NAMES {
        let body = room
            .view("OWNER", &path(view, &format!("branch=MAIN&since={before}")))
            .await;
        assert_eq!(body["version"], json!(now), "{what}: {view}: {body}");
        assert_eq!(body["changed"], json!(true), "{what}: {view}: {body}");
        assert!(
            body.get("data").is_some_and(|d| !d.is_null()),
            "{what}: {view}: data in {body}"
        );
    }
}

fn ids(rows: &Value, key: &str) -> Vec<String> {
    rows.as_array()
        .unwrap_or_else(|| panic!("a page is an array: {rows}"))
        .iter()
        .map(|r| {
            r[key]
                .as_str()
                .unwrap_or_else(|| panic!("{key} is text in {r}"))
                .to_owned()
        })
        .collect()
}

// ── who may read ─────────────────────────────────────────────────────────────

/// Nobody signed in: each view answers the policy's refusal as a v3 refusal (§8).
#[actix_web::test]
async fn nobody_signed_in_is_asked_to_sign_in_by_every_view() {
    let room = Room::open().await;
    for view in VIEW_NAMES {
        let answer = room.get("", &path(view, "branch=MAIN")).await;
        assert_eq!(answer.status, 401, "{view}: {:.300}", answer.text);
        assert_eq!(
            answer.json(),
            json!({ "error": { "code": "signIn", "message": SIGN_IN } }),
            "{view}"
        );
    }
    room.finish().await;
}

/// V1: the four views answer exactly whom `Policy::allow(_, Devices, "snapshot")` answers,
/// for every caller kind, and on the branch `Policy::branch` gives (staff: their own).
#[actix_web::test]
async fn every_view_answers_exactly_whom_the_devices_snapshot_row_allows() {
    let room = Room::open().await;
    let mut callers: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        ("OWNER".to_string(), Some(Actor::owner("owner"))),
    ];
    for role in Role::STAFF {
        let staff = Actor::staff("employee", role, BranchId::new(MAIN));
        callers.push((role.as_str().to_string(), staff));
        // the owner and an employee in one browser: a view not on StaffRpc answers the owner
        callers.push((
            format!("OWNER+{}", role.as_str()),
            Some(Actor::owner("owner")),
        ));
    }
    let named = BranchId::new(B9);
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in &callers {
        let policy = Policy::allow(actor.as_ref(), EntryPath::Devices, &Action::new("snapshot"));
        let branch = Policy::branch(actor.as_ref(), Some(&named));
        for view in VIEW_NAMES {
            let answer = room.get(who, &path(view, "branch=B9")).await;
            let what = format!("{view} as {who:?}");
            match &policy {
                Ok(()) => {
                    allowed += 1;
                    assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                    let body = answer.json();
                    assert_eq!(body["changed"], json!(true), "{what}: {body}");
                    if view == "devices" {
                        assert_eq!(
                            body["data"]["branchId"],
                            json!(branch.as_str()),
                            "{what}: the branch Policy::branch gives"
                        );
                    } else {
                        assert!(body["data"].is_array(), "{what}: a page: {body}");
                    }
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
    }
    assert_eq!(
        allowed,
        4 * 19,
        "every signed-in caller reads all four views"
    );
    assert_eq!(refused, 4, "nobody is refused by all four");
    room.finish().await;
}

/// A staff member naming another branch is answered for their own, `branchId` says which,
/// and the pages hold their own branch's rows only.
#[actix_web::test]
async fn a_staff_member_naming_another_branch_is_answered_for_their_own() {
    let room = Room::open().await;
    let main_job = room.queue_drawer("OWNER", MAIN).await;
    let b9_job = room.queue_drawer("OWNER", B9).await;
    room.open_shift(B9, json!({ "THB": 100000 })).await;
    let b9_exchange = room.save(B9, "BUY", "1", "0", "LIVE").await;

    let devices = room.data("FRONT", &path("devices", "branch=B9")).await;
    assert_eq!(devices["branchId"], json!(MAIN), "FRONT works in MAIN");
    assert_eq!(
        devices["completedCount"],
        json!(0),
        "B9's exchange is not MAIN's"
    );
    let jobs = room.data("FRONT", &path("device_jobs", "branch=B9")).await;
    assert_eq!(ids(&jobs, "id"), vec![main_job.clone()], "MAIN's job only");
    let events = room
        .data("FRONT", &path("device_events", "branch=B9"))
        .await;
    let mains: Vec<String> = sqlx::query_scalar(
        "SELECT e.seq::text FROM device_events e WHERE e.branch_id = 'MAIN' ORDER BY e.seq DESC",
    )
    .fetch_all(&room.desk.db.pool)
    .await
    .expect("read MAIN's lines");
    assert!(!mains.is_empty(), "the queue wrote a line in MAIN");
    assert_eq!(ids(&events, "seq"), mains, "MAIN's lines only");
    let picker = room
        .data("FRONT", &path("device_transactions", "branch=B9"))
        .await;
    assert_eq!(picker, json!([]), "MAIN has no exchange");

    // the owner reads the branch they name
    let owners = room.data("OWNER", &path("device_jobs", "branch=B9")).await;
    assert_eq!(ids(&owners, "id"), vec![b9_job]);
    let owners = room
        .data("OWNER", &path("device_transactions", "branch=B9"))
        .await;
    assert_eq!(ids(&owners, "id"), vec![b9_exchange]);

    // and FRONT's since is compared with MAIN's version
    let v = room.version(MAIN).await;
    let body = room
        .view("FRONT", &path("devices", &format!("branch=B9&since={v}")))
        .await;
    is_unchanged(&body, v, "FRONT's since is MAIN's version");
    room.finish().await;
}

// ── only what changed (§4.2) ─────────────────────────────────────────────────

/// With `since` equal to the branch's version each view answers `changed: false` and is not
/// loaded: with every table a view could read renamed away, `since` still answers, while the
/// same read without `since` fails (so it would have been loaded).
#[actix_web::test]
async fn an_equal_since_is_unchanged_and_no_view_is_loaded() {
    let room = Room::open().await;
    room.queue_drawer("OWNER", MAIN).await;
    let v = room.version(MAIN).await;
    assert!(v > 0, "the queue moved MAIN's version");
    for view in VIEW_NAMES {
        let body = room
            .view("OWNER", &path(view, &format!("branch=MAIN&since={v}")))
            .await;
        is_unchanged(&body, v, view);
    }

    room.hide_the_views_tables().await;
    for view in VIEW_NAMES {
        for _ in 0..3 {
            let body = room
                .view("OWNER", &path(view, &format!("branch=MAIN&since={v}")))
                .await;
            is_unchanged(&body, v, &format!("{view} with its tables gone"));
        }
        let loaded = room.get("OWNER", &path(view, "branch=MAIN")).await;
        assert_eq!(
            loaded.status, 500,
            "{view}: read with no since, it is loaded and finds no table: {:.300}",
            loaded.text
        );
    }
    room.finish().await;
}

/// After one write the same `since` reports `changed: true` with the written row.
#[actix_web::test]
async fn after_a_write_the_same_since_is_changed_with_data() {
    let room = Room::open().await;
    room.open_shift(MAIN, json!({ "THB": 100000 })).await;
    let v = room.version(MAIN).await;
    for view in VIEW_NAMES {
        let body = room
            .view("OWNER", &path(view, &format!("branch=MAIN&since={v}")))
            .await;
        is_unchanged(&body, v, view);
    }

    let job = room.queue_drawer("FRONT", MAIN).await;
    let body = room
        .view(
            "OWNER",
            &path("device_jobs", &format!("branch=MAIN&since={v}")),
        )
        .await;
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(ids(&body["data"], "id"), vec![job.clone()]);
    let body = room
        .view(
            "OWNER",
            &path("device_events", &format!("branch=MAIN&since={v}")),
        )
        .await;
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(body["data"][0]["kind"], json!("DRAWER_REQUEST"), "{body}");

    let v = room.version(MAIN).await;
    let exchange = room.save(MAIN, "BUY", "100", "0", "LIVE").await;
    for view in ["devices", "device_transactions"] {
        let body = room
            .view("OWNER", &path(view, &format!("branch=MAIN&since={v}")))
            .await;
        assert_eq!(body["changed"], json!(true), "{view}: {body}");
        assert_eq!(body["version"], json!(room.version(MAIN).await), "{view}");
    }
    let picker = room
        .data("OWNER", &path("device_transactions", "branch=MAIN"))
        .await;
    assert_eq!(ids(&picker, "id"), vec![exchange]);
    let devices = room.data("OWNER", &path("devices", "branch=MAIN")).await;
    assert_eq!(devices["completedCount"], json!(1));
    room.finish().await;
}

/// V4 (§4.2): one write by each writer of the views' tables, then the same `since` reports
/// `changed: true` on all four views. Writers: `/api/devices` `setPrinter`, `setDrawer`,
/// `printerOnline`, `queue`, `process`, `retry`, `drawerObservation`; the owner's agent
/// pairing on `/api/device-setup` (`add`, `newCode`, `revoke`; the agent's own `pair` is only
/// the way to a token here — it changes nothing a view reads) and the agent's `claim` and
/// `report`; a kiosk audit line (`/api/kiosk` `configure`); shift open, move stock and
/// close on `/api/rpc`; and a saved exchange.
#[actix_web::test]
async fn every_writer_of_the_views_tables_moves_the_version_they_answer() {
    let room = Room::open().await;

    let v = room.version(MAIN).await;
    room.devices("OWNER", "setPrinter", json!({ "kind": "SIMULATED" }))
        .await;
    every_view_changed(&room, v, "setPrinter").await;

    let v = room.version(MAIN).await;
    room.devices(
        "OWNER",
        "setDrawer",
        json!({ "printerModel": NEOCAL, "hasDrawer": true }),
    )
    .await;
    every_view_changed(&room, v, "setDrawer").await;

    // the counter agent: added, given a new code, paired, claims and reports, revoked
    let v = room.version(MAIN).await;
    let added = room
        .ok(
            "OWNER",
            SETUP,
            "add",
            json!({ "branchId": MAIN, "name": "Mac 1" }),
        )
        .await;
    every_view_changed(&room, v, "device-setup add").await;
    let agent = added["agentId"].as_str().expect("an agent id").to_owned();

    let v = room.version(MAIN).await;
    let renewed = room
        .ok("OWNER", SETUP, "newCode", json!({ "agentId": agent }))
        .await;
    every_view_changed(&room, v, "device-setup newCode").await;
    let code = renewed["code"].as_str().expect("a code").to_owned();

    room.tick();
    let paired = room
        .app
        .send(
            Call::post(
                ORIGIN,
                PAIR,
                json!({ "code": code, "host": "counter-1.local", "os": "macos-14.6-arm64",
                        "version": "1.0.0", "build": "abc1234" })
                .to_string(),
            )
            .without("origin")
            .without("x-xchange-request")
            .header("x-real-ip", "10.1.1.1"),
        )
        .await;
    assert_eq!(paired.status, 200, "pair: {:.300}", paired.text);
    let token = paired.json()["token"].as_str().expect("a token").to_owned();

    let agent_call = |route: &str, body: Value| {
        Call::post(ORIGIN, route, body.to_string())
            .without("origin")
            .without("x-xchange-request")
            .header("authorization", &format!("Bearer {token}"))
            .header("x-real-ip", "10.1.1.1")
    };
    room.queue_drawer("FRONT", MAIN).await;
    let v = room.version(MAIN).await;
    room.tick();
    let claim = room
        .app
        .send(agent_call(
            CLAIM,
            json!({ "waitMs": 0, "version": "1.0.1", "build": "def5678",
                    "os": "macos-14.6-arm64", "host": "counter-1.local",
                    "drawer": { "transport": "CUPS", "target": "POS80_" } }),
        ))
        .await;
    assert_eq!(claim.status, 200, "claim: {:.300}", claim.text);
    every_view_changed(&room, v, "device-agent claim").await;
    let claimed = claim.json();

    let v = room.version(MAIN).await;
    room.tick();
    let report = room
        .app
        .send(agent_call(
            REPORT,
            json!({ "jobId": claimed["jobId"], "leaseId": claimed["leaseId"],
                    "outcome": "DRAWER_OPENED", "reason": "",
                    "detail": { "cupsJob": "POS80_-12" } }),
        ))
        .await;
    assert!(
        (200..300).contains(&report.status),
        "report: {} {:.300}",
        report.status,
        report.text
    );
    every_view_changed(&room, v, "device-agent report").await;

    let v = room.version(MAIN).await;
    room.ok("OWNER", SETUP, "revoke", json!({ "agentId": agent }))
        .await;
    every_view_changed(&room, v, "device-setup revoke").await;

    let v = room.version(MAIN).await;
    room.devices("BACKEND", "printerOnline", json!({ "online": false }))
        .await;
    every_view_changed(&room, v, "printerOnline").await;

    let v = room.version(MAIN).await;
    let job = room.queue_drawer("FRONT", MAIN).await;
    every_view_changed(&room, v, "queue").await;

    let v = room.version(MAIN).await;
    let processed = room.devices("FRONT", "process", json!({ "id": job })).await;
    assert_eq!(processed["status"], json!("FAILED"), "the printer is off");
    every_view_changed(&room, v, "process").await;

    let v = room.version(MAIN).await;
    room.devices("FRONT", "retry", json!({ "id": job })).await;
    every_view_changed(&room, v, "retry").await;

    let v = room.version(MAIN).await;
    room.devices(
        "FRONT",
        "drawerObservation",
        json!({ "ref": "", "state": "OPEN" }),
    )
    .await;
    every_view_changed(&room, v, "drawerObservation").await;

    // a kiosk audit line goes into device_events (#603: the won is published first, which
    // moves no version)
    publication_seed::publish(
        &room.desk.db.pool,
        MAIN,
        chrono::Utc::now(),
        &[publication_seed::both("KRW", "23.1", "25.9")],
    )
    .await;
    let v = room.version(MAIN).await;
    room.ok(
        "OWNER",
        KIOSK,
        "configure",
        json!({ "code": "KRW", "enabled": true, "buy": 0.0231, "sell": 0.0259 }),
    )
    .await;
    every_view_changed(&room, v, "kiosk configure").await;

    // the shift and an exchange: the balance, the count and the picker
    let v = room.version(MAIN).await;
    room.open_shift(MAIN, json!({ "THB": 100000, "USD": 1000 }))
        .await;
    every_view_changed(&room, v, "adminOpenShift").await;

    let v = room.version(MAIN).await;
    let shift = room.open_shift_id(MAIN).await;
    room.ok(
        "OWNER",
        RPC,
        "adminMoveStock",
        json!({ "branchId": MAIN, "requestId": request_id(), "shiftId": shift,
                "currency": "USD", "direction": "IN", "amount": 10, "reason": "from the safe" }),
    )
    .await;
    every_view_changed(&room, v, "adminMoveStock").await;

    let v = room.version(MAIN).await;
    room.save(MAIN, "BUY", "100", "0", "LIVE").await;
    every_view_changed(&room, v, "a saved exchange").await;

    let v = room.version(MAIN).await;
    room.close_shift(MAIN, json!({ "THB": "96790.00", "USD": "1110.00" }))
        .await;
    every_view_changed(&room, v, "adminCloseShift").await;
    room.finish().await;
}

// ── the devices view (§7.1, §7.5) ────────────────────────────────────────────

/// The shape of `data`, and what a branch nobody has touched answers.
#[actix_web::test]
async fn the_devices_view_answers_the_tickets_fields() {
    let room = Room::open().await;
    let data = room.data("OWNER", &path("devices", "branch=MAIN")).await;
    assert_eq!(
        keys(&data),
        set_of(&DEVICES_KEYS),
        "the keys of data: {data}"
    );
    assert_eq!(data["branchId"], json!(MAIN));
    assert_eq!(data["printerOnline"], json!(true), "nobody switched it off");
    assert_eq!(data["printerKind"], json!("SIMULATED"));
    assert_eq!(data["printerModel"], json!(NEOCAL));
    assert_eq!(data["hasDrawer"], Value::Null, "nobody said");
    assert_eq!(data["drawerAgentActive"], json!(false));
    assert_eq!(
        data["balanceThb"],
        json!("0.00"),
        "no shift: zero, two decimals"
    );
    assert_eq!(data["balanceUsd"], json!("0.00"));
    assert_eq!(data["completedCount"], json!(0));

    // every branch, as the admin snapshot lists them today (snapshot.rs:95)
    let admin = room
        .ok("OWNER", RPC, "adminSnapshot", json!({ "branchId": MAIN }))
        .await;
    let listed: Vec<Value> = admin["branches"]
        .as_array()
        .expect("the snapshot's branches")
        .iter()
        .map(|b| json!({ "id": b["id"], "name": b["name"] }))
        .collect();
    assert_eq!(data["branches"], Value::Array(listed), "{data}");
    let named: BTreeSet<(String, String)> = data["branches"]
        .as_array()
        .expect("branches")
        .iter()
        .map(|b| {
            assert_eq!(keys(b), set_of(&["id", "name"]), "a branch: {b}");
            (
                b["id"].as_str().expect("id").to_owned(),
                b["name"].as_str().expect("name").to_owned(),
            )
        })
        .collect();
    assert!(
        named.contains(&(MAIN.to_owned(), MAIN_NAME.to_owned())),
        "{named:?}"
    );
    assert!(
        named.contains(&(B9.to_owned(), B9_NAME.to_owned())),
        "{named:?}"
    );

    // a staff member sees every branch too, and is answered for their own
    let staff = room.data("SYSTEM", &path("devices", "")).await;
    assert_eq!(staff["branchId"], json!(MAIN));
    assert_eq!(staff["branches"], data["branches"]);
    room.finish().await;
}

/// V5: the printer kind, the drawer setting and whether a device agent is active follow the
/// writes that set them, for the page to describe what a job will do.
#[actix_web::test]
async fn the_printer_the_drawer_and_the_agent_follow_their_writes() {
    let room = Room::open().await;
    let main = path("devices", "branch=MAIN");
    let read = || room.data("OWNER", &main);

    room.devices(
        "OWNER",
        "setPrinter",
        json!({ "kind": "NETWORK", "host": "10.0.0.5", "port": 9100 }),
    )
    .await;
    let data = read().await;
    assert_eq!(data["printerKind"], json!("NETWORK"));
    assert_eq!(
        keys(&data),
        set_of(&DEVICES_KEYS),
        "no host or port key: {data}"
    );

    room.devices("BACKEND", "printerOnline", json!({ "online": false }))
        .await;
    assert_eq!(read().await["printerOnline"], json!(false));

    room.devices(
        "OWNER",
        "setDrawer",
        json!({ "printerModel": NEOCAL, "hasDrawer": true }),
    )
    .await;
    let data = read().await;
    assert_eq!(data["hasDrawer"], json!(true));
    assert_eq!(data["printerModel"], json!(NEOCAL));
    room.devices(
        "OWNER",
        "setDrawer",
        json!({ "printerModel": NEOCAL, "hasDrawer": false }),
    )
    .await;
    assert_eq!(read().await["hasDrawer"], json!(false));

    room.devices("OWNER", "setPrinter", json!({ "kind": "SIMULATED" }))
        .await;
    room.devices("OWNER", "printerOnline", json!({ "online": true }))
        .await;
    let data = read().await;
    assert_eq!(data["printerKind"], json!("SIMULATED"));
    assert_eq!(data["printerOnline"], json!(true));

    // an agent added (paired or not, not revoked) is active; revoked, it is not
    let added = room
        .ok(
            "OWNER",
            SETUP,
            "add",
            json!({ "branchId": MAIN, "name": "Mac 1" }),
        )
        .await;
    assert_eq!(read().await["drawerAgentActive"], json!(true));
    let b9 = room.data("OWNER", &path("devices", "branch=B9")).await;
    assert_eq!(
        b9["drawerAgentActive"],
        json!(false),
        "MAIN's agent is not B9's"
    );
    room.ok(
        "OWNER",
        SETUP,
        "revoke",
        json!({ "agentId": added["agentId"] }),
    )
    .await;
    assert_eq!(read().await["drawerAgentActive"], json!(false));
    room.finish().await;
}

/// §7.5: no shift → zero; an open shift → its `expected`; the last shift closed → its
/// `counted`. Two decimals, as decimal strings.
#[actix_web::test]
async fn the_balance_is_the_open_shifts_expected_or_the_closed_shifts_counted() {
    let room = Room::open().await;
    let balance = || async {
        let data = room.data("OWNER", &path("devices", "branch=MAIN")).await;
        (data["balanceThb"].clone(), data["balanceUsd"].clone())
    };
    assert_eq!(balance().await, (json!("0.00"), json!("0.00")), "no shift");

    room.open_shift(MAIN, json!({ "THB": 5000, "USD": "10.5" }))
        .await;
    assert_eq!(
        balance().await,
        (json!("5000.00"), json!("10.50")),
        "the opening"
    );

    // a cash purchase of 100 USD at 32.1 pays 3,210 baht out
    room.save(MAIN, "BUY", "100", "0", "LIVE").await;
    assert_eq!(
        balance().await,
        (json!("1790.00"), json!("110.50")),
        "the open shift's expected"
    );
    // a cash sale of 10 USD at 33 with a 20 baht fee takes 350 baht in
    room.save(MAIN, "SELL", "10", "20", "LIVE").await;
    assert_eq!(balance().await, (json!("2140.00"), json!("100.50")));
    assert_eq!(
        room.expected_of_open_shift(MAIN).await,
        json!({ "THB": "2140.00", "USD": "100.50" }),
        "the server's own figure"
    );

    room.close_shift(MAIN, json!({ "THB": "2139.75", "USD": "100.50" }))
        .await;
    assert_eq!(
        balance().await,
        (json!("2139.75"), json!("100.50")),
        "the closed shift's counted, not its expected"
    );

    room.open_shift(MAIN, json!({ "THB": "33250" })).await;
    assert_eq!(
        balance().await,
        (json!("33250.00"), json!("0.00")),
        "the open shift again; a currency it does not hold is zero"
    );
    room.finish().await;
}

/// The metric is the server's stored `expected`: changed behind the routes, the view shows
/// the stored figure, which no sum of opening + exchanges + movements gives.
#[actix_web::test]
async fn the_balance_is_the_shifts_stored_figure_never_a_recomputation() {
    let room = Room::open().await;
    room.open_shift(MAIN, json!({ "THB": 5000, "USD": 100 }))
        .await;
    room.save(MAIN, "BUY", "100", "0", "LIVE").await;
    let shift = room.open_shift_id(MAIN).await;
    sqlx::query(
        "UPDATE shift_balances SET expected = 4321.09 WHERE shift_id = $1 AND code = 'THB'",
    )
    .bind(&shift)
    .execute(&room.desk.db.pool)
    .await
    .expect("change the stored figure");
    let data = room.data("OWNER", &path("devices", "branch=MAIN")).await;
    assert_eq!(data["balanceThb"], json!("4321.09"), "{data}");
    assert_eq!(data["balanceUsd"], json!("200.00"), "{data}");
    room.finish().await;
}

/// The completed-exchange count is the branch's LIVE exchanges (the ones a print job may
/// name, `pg_saved_exchanges.rs`); DEMO ones and other branches' are not counted.
#[actix_web::test]
async fn the_completed_count_is_the_branchs_live_exchanges() {
    let room = Room::open().await;
    room.open_shift(MAIN, json!({ "THB": 100000 })).await;
    room.open_shift(B9, json!({ "THB": 100000 })).await;
    room.save(MAIN, "BUY", "1", "0", "LIVE").await;
    room.save(MAIN, "BUY", "2", "0", "LIVE").await;
    room.save(B9, "BUY", "3", "0", "LIVE").await;
    room.save(MAIN, "BUY", "4", "0", "DEMO").await;
    let data = room.data("OWNER", &path("devices", "branch=MAIN")).await;
    assert_eq!(data["completedCount"], json!(2), "{data}");
    let data = room.data("OWNER", &path("devices", "branch=B9")).await;
    assert_eq!(data["completedCount"], json!(1), "{data}");
    room.finish().await;
}

// ── the pages (§7.2–§7.4) ────────────────────────────────────────────────────

/// 101 jobs: 50 newest by default, 100 at most, `before=<100th>` gives the 101st; a limit
/// over 100 is refused.
#[actix_web::test]
async fn device_jobs_pages_newest_first_fifty_by_default_and_a_hundred_at_most() {
    let room = Room::open().await;
    let mut queued = Vec::new();
    for _ in 0..101 {
        queued.push(room.queue_drawer("FRONT", MAIN).await);
    }
    let newest_first: Vec<String> = queued.iter().rev().cloned().collect();

    let first = room
        .data("OWNER", &path("device_jobs", "branch=MAIN"))
        .await;
    assert_eq!(
        ids(&first, "id"),
        newest_first[..50].to_vec(),
        "50 by default"
    );
    let hundred = room
        .data("OWNER", &path("device_jobs", "branch=MAIN&limit=100"))
        .await;
    assert_eq!(ids(&hundred, "id"), newest_first[..100].to_vec());
    let older = room
        .data(
            "OWNER",
            &path(
                "device_jobs",
                &format!("branch=MAIN&limit=100&before={}", newest_first[99]),
            ),
        )
        .await;
    assert_eq!(ids(&older, "id"), vec![queued[0].clone()], "the 101st");
    let three = room
        .data(
            "OWNER",
            &path(
                "device_jobs",
                &format!("branch=MAIN&limit=3&before={}", newest_first[9]),
            ),
        )
        .await;
    assert_eq!(ids(&three, "id"), newest_first[10..13].to_vec());

    let over = room
        .get("OWNER", &path("device_jobs", "branch=MAIN&limit=101"))
        .await;
    assert_eq!(over.status, 400, "a limit over 100: {:.200}", over.text);
    room.finish().await;
}

/// 101 lines: the same paging, newest first by `seq`, `before` = a `seq`.
#[actix_web::test]
async fn device_events_pages_newest_first_by_seq() {
    let room = Room::open().await;
    for _ in 0..101 {
        room.devices(
            "FRONT",
            "drawerObservation",
            json!({ "ref": "", "state": "OPEN" }),
        )
        .await;
    }
    let seqs: Vec<String> = sqlx::query_scalar(
        "SELECT e.seq::text FROM device_events e WHERE e.branch_id = 'MAIN' ORDER BY e.seq DESC",
    )
    .fetch_all(&room.desk.db.pool)
    .await
    .expect("read the lines");
    assert_eq!(seqs.len(), 101);

    let first = room
        .data("OWNER", &path("device_events", "branch=MAIN"))
        .await;
    assert_eq!(ids(&first, "seq"), seqs[..50].to_vec(), "50 by default");
    let hundred = room
        .data("OWNER", &path("device_events", "branch=MAIN&limit=100"))
        .await;
    assert_eq!(ids(&hundred, "seq"), seqs[..100].to_vec());
    let older = room
        .data(
            "OWNER",
            &path(
                "device_events",
                &format!("branch=MAIN&limit=100&before={}", seqs[99]),
            ),
        )
        .await;
    assert_eq!(ids(&older, "seq"), vec![seqs[100].clone()], "the 101st");
    let over = room
        .get("OWNER", &path("device_events", "branch=MAIN&limit=101"))
        .await;
    assert_eq!(over.status, 400, "a limit over 100: {:.200}", over.text);
    room.finish().await;
}

/// 101 LIVE exchanges: the picker pages them newest first; a DEMO exchange is not offered.
#[actix_web::test]
async fn device_transactions_pages_the_live_exchanges_newest_first() {
    let room = Room::open().await;
    room.open_shift(MAIN, json!({ "THB": 1000000 })).await;
    let mut saved = Vec::new();
    for _ in 0..101 {
        saved.push(room.save(MAIN, "BUY", "1", "0", "LIVE").await);
    }
    let demo = room.save(MAIN, "BUY", "1", "0", "DEMO").await;
    let newest_first: Vec<String> = saved.iter().rev().cloned().collect();

    let first = room
        .data("OWNER", &path("device_transactions", "branch=MAIN"))
        .await;
    assert_eq!(
        ids(&first, "id"),
        newest_first[..50].to_vec(),
        "50 by default"
    );
    let hundred = room
        .data(
            "OWNER",
            &path("device_transactions", "branch=MAIN&limit=100"),
        )
        .await;
    assert_eq!(ids(&hundred, "id"), newest_first[..100].to_vec());
    assert!(!ids(&hundred, "id").contains(&demo), "no DEMO exchange");
    let older = room
        .data(
            "OWNER",
            &path(
                "device_transactions",
                &format!("branch=MAIN&limit=100&before={}", newest_first[99]),
            ),
        )
        .await;
    assert_eq!(ids(&older, "id"), vec![saved[0].clone()], "the 101st");
    let over = room
        .get(
            "OWNER",
            &path("device_transactions", "branch=MAIN&limit=101"),
        )
        .await;
    assert_eq!(over.status, 400, "a limit over 100: {:.200}", over.text);
    room.finish().await;
}

// ── the rows (§7.2–§7.4, §8) ─────────────────────────────────────────────────

/// A job row is `{id, kind, ref, status, actor, created}`, the same values the devices
/// snapshot answers today, `created` with milliseconds.
#[actix_web::test]
async fn a_job_row_is_what_the_devices_snapshot_shows_of_it() {
    let room = Room::open().await;
    room.open_shift(MAIN, json!({ "THB": 100000 })).await;
    let exchange = room.save(MAIN, "BUY", "100", "0", "LIVE").await;
    room.devices(
        "FRONT",
        "queue",
        json!({ "kind": "PRINT_RECEIPT", "ref": exchange, "requestId": request_id() }),
    )
    .await;
    room.queue_drawer("BACKEND", MAIN).await;

    let rows = room
        .data("OWNER", &path("device_jobs", "branch=MAIN"))
        .await;
    let snapshot = room.devices("OWNER", "snapshot", json!({})).await;
    let jobs = snapshot["jobs"].as_array().expect("the snapshot's jobs");
    let rows = rows.as_array().expect("a page");
    assert_eq!(rows.len(), jobs.len());
    for (row, job) in rows.iter().zip(jobs) {
        assert_eq!(keys(row), set_of(&JOB_KEYS), "a job row: {row}");
        for key in ["id", "kind", "ref", "status", "actor", "created"] {
            assert_eq!(row[key], job[key], "{key} of {row}");
        }
        let created = row["created"].as_str().expect("created is text");
        assert!(
            created.len() == 24 && created.ends_with('Z') && created.as_bytes()[19] == b'.',
            "RFC 3339 with milliseconds: {created:?}"
        );
    }
    assert_eq!(
        rows[1]["ref"],
        json!(exchange),
        "the print job names the exchange"
    );
    room.finish().await;
}

/// An event row is `{seq, kind, ref, at, actor, detail}`: `seq` as text, `detail` the compact
/// JSON text of what the devices snapshot answers — amount keys as `list_events` gives them.
#[actix_web::test]
async fn an_event_row_is_what_the_devices_snapshot_shows_of_it() {
    let room = Room::open().await;
    room.devices(
        "FRONT",
        "drawerObservation",
        json!({ "ref": "", "state": "CLOSED" }),
    )
    .await;
    room.queue_drawer("FRONT", MAIN).await;
    sqlx::query(
        "INSERT INTO device_events (branch_id, actor, kind, ref, detail, at) VALUES \
         ('MAIN', 'front1', 'COUNT_CONFIRMED', 'XC-20261006-000001', \
          '{\"thb\": 1234.5, \"foreignAmount\": 100, \"note\": \"x\"}'::jsonb, $1)",
    )
    .bind(room.tick())
    .execute(&room.desk.db.pool)
    .await
    .expect("a line with amount keys");

    let rows = room
        .data("OWNER", &path("device_events", "branch=MAIN"))
        .await;
    let snapshot = room.devices("OWNER", "snapshot", json!({})).await;
    let events = snapshot["events"].as_array().expect("the snapshot's lines");
    let rows = rows.as_array().expect("a page");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.len(), events.len());
    let seqs: Vec<String> = sqlx::query_scalar(
        "SELECT e.seq::text FROM device_events e WHERE e.branch_id = 'MAIN' ORDER BY e.seq DESC",
    )
    .fetch_all(&room.desk.db.pool)
    .await
    .expect("read the lines");
    for ((row, event), seq) in rows.iter().zip(events).zip(&seqs) {
        assert_eq!(keys(row), set_of(&EVENT_KEYS), "an event row: {row}");
        assert_eq!(row["seq"], json!(seq), "seq is the keyset id, as text");
        for key in ["kind", "ref", "at", "actor"] {
            assert_eq!(row[key], event[key], "{key} of {row}");
        }
        let text = row["detail"]
            .as_str()
            .unwrap_or_else(|| panic!("detail is JSON text: {row}"));
        assert!(!text.contains('\n'), "compact: {text:?}");
        let detail: Value = serde_json::from_str(text).expect("detail parses");
        assert_eq!(detail, event["detail"], "detail of {row}");
    }
    let counted: Value =
        serde_json::from_str(rows[0]["detail"].as_str().expect("text")).expect("detail parses");
    assert_eq!(
        counted["thb"],
        json!("1234.5"),
        "an amount key is text: {counted}"
    );
    assert_eq!(counted["foreignAmount"], json!("100"), "{counted}");
    assert_eq!(counted["note"], json!("x"));
    room.finish().await;
}

/// A picker row is `{id, type, amount, currency}`, `amount` a decimal string (W7).
#[actix_web::test]
async fn a_transaction_row_carries_its_amount_as_a_decimal_string() {
    let room = Room::open().await;
    room.open_shift(MAIN, json!({ "THB": 100000, "USD": 1000 }))
        .await;
    let buy = room.save(MAIN, "BUY", "100", "0", "LIVE").await;
    let sell = room.save(MAIN, "SELL", "12.5", "20", "LIVE").await;
    let rows = room
        .data("OWNER", &path("device_transactions", "branch=MAIN"))
        .await;
    let rows = rows.as_array().expect("a page");
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(keys(row), set_of(&TRANSACTION_KEYS), "a picker row: {row}");
        assert_eq!(row["currency"], json!("USD"));
    }
    assert_eq!(rows[0]["id"], json!(sell));
    assert_eq!(rows[0]["type"], json!("SELL"));
    assert_eq!(decimal(&rows[0]["amount"], "SELL"), Decimal::new(125, 1));
    assert_eq!(rows[1]["id"], json!(buy));
    assert_eq!(rows[1]["type"], json!("BUY"));
    assert_eq!(decimal(&rows[1]["amount"], "BUY"), Decimal::new(100, 0));
    room.finish().await;
}

// ── the generated types (§5.1, done criterion 2) ─────────────────────────────

/// A new empty directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "xc-v3-devices-types-{}-{}",
            std::process::id(),
            next()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exported(name: &str) -> String {
    let view = VIEWS.iter().find(|v| v.name == name).unwrap_or_else(|| {
        panic!(
            "{name} is in the generated view table: {:?}",
            VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
        )
    });
    let scratch = Scratch::new();
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {name}: {e}"));
    std::fs::read_to_string(scratch.0.join(format!("{name}.ts")))
        .unwrap_or_else(|e| panic!("{name} writes {name}.ts: {e}"))
}

fn has_field(ts: &str, field: &str, ty: &str, file: &str) {
    let wanted = format!("{field}: {ty}");
    assert!(ts.contains(&wanted), "{file}: `{wanted}` in:\n{ts}");
}

#[test]
fn the_four_views_are_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    for name in VIEW_NAMES {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
}

/// The generated TypeScript carries the ticket's fields, money as `Money` (a decimal string,
/// `common.ts`), counts as numbers and never `bigint`.
#[test]
fn the_generated_types_carry_the_tickets_fields() {
    let ts = exported("devices");
    for (field, ty) in [
        ("branchId", "string"),
        ("printerOnline", "boolean"),
        ("printerKind", "string"),
        ("printerModel", "string"),
        ("hasDrawer", "boolean | null"),
        ("drawerAgentActive", "boolean"),
        ("balanceThb", "Money"),
        ("balanceUsd", "Money"),
        ("completedCount", "number"),
        ("id", "string"),
        ("name", "string"),
    ] {
        has_field(&ts, field, ty, "devices.ts");
    }
    assert!(
        ts.contains("branches: Array<"),
        "devices.ts: branches:\n{ts}"
    );

    let ts = exported("device_jobs");
    for field in JOB_KEYS {
        has_field(&ts, field, "string", "device_jobs.ts");
    }
    let ts = exported("device_events");
    for field in EVENT_KEYS {
        has_field(&ts, field, "string", "device_events.ts");
    }
    let ts = exported("device_transactions");
    for (field, ty) in [
        ("id", "string"),
        ("type", "string"),
        ("amount", "Money"),
        ("currency", "string"),
    ] {
        has_field(&ts, field, ty, "device_transactions.ts");
    }
    for name in VIEW_NAMES {
        let ts = exported(name);
        assert!(
            !ts.contains("bigint"),
            "{name}.ts: numbers, never bigint:\n{ts}"
        );
        assert!(!ts.contains("number | string"), "{name}.ts:\n{ts}");
    }
}

/// The four files are committed under `web/src/shared/api/generated/` exactly as the DTOs
/// generate them (the drift gate of `v3_types_test.rs` covers them once they are there).
#[test]
fn the_four_generated_files_are_committed() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/shared/api/generated");
    for name in VIEW_NAMES {
        let committed = std::fs::read_to_string(dir.join(format!("{name}.ts")))
            .unwrap_or_else(|e| panic!("web/src/shared/api/generated/{name}.ts: {e}"));
        assert_eq!(
            committed,
            exported(name),
            "{name}.ts is what the DTOs generate"
        );
    }
}
