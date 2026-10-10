//! Ticket #537 (DK-A, `docs/design/xchange-v3-wave3-desk.md` §4.1, §6, §10, PR 536): the desk's one
//! read. Written before the implementation by the ticket's preparer (claude/b060e9). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/desk_config?branch=&since=` — a `BranchView`, `NoFilter`, inheriting
//!   `(EntryPath::Rpc, "adminDeskConfig")`.
//! - `GET /api/v3/views/staff_desk_config?branch=&since=` — the same, inheriting
//!   `(EntryPath::StaffRpc, "adminDeskConfig")` (FRONT only, `policy.rs:135`), answered for the
//!   staff session (`view.rs:339-341`).
//! - `data` = `{branchId, branchName, shiftOpen, adminUrl, rates, paymentMethods}` (§4.1). It is
//!   what the desk reads from v2's `adminDeskConfig` (`snapshot.rs:144-167`) **without** the
//!   custody log (`operations`) and without the shift id (`activeShift` becomes `shiftOpen`).
//! - `rates[]` = `{code, unitSize, buy, sell, publication}`. The rate model is the Blueprint's
//!   (owner 2026-10-09, PR 527 issuecomment-6073063976; design D-12): `unitSize` and `publication`
//!   are typed "per the Blueprint rate design (pending, #547/#548)", so this file pins that they
//!   are present and filled, never their shape. `buy` and `sell` are the published rate as a
//!   **decimal string** (W7), equal as a number to the rate the server holds, or `null` for a side
//!   with no rate. No number of decimal places is pinned (D-12: no `toFixed(4)`, no 4-place
//!   `Rate`).
//!
//! **Seeding rates.** On `main` the only way to put a rate in a branch is v2's direct save
//! (`/api/rpc` `adminSaveRate`). The Blueprint replaces it with draft → approve → publish (#548).
//! Every seed goes through one helper, [`Room::publish`]. When #548 lands, the preparer re-points
//! that helper at the publication path; no assertion changes. The comparison source
//! ([`Room::server_rates`]) is read the same way: today `adminDeskConfig`'s `rates`, later the
//! branch's published rates.
//!
//! Everything goes through the server's whole route table with real sessions made at the real
//! sign-in doors (`rpc_common`). Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;
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
use xchange::presentation::http::v3::VIEWS;

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";
const OWNER_VIEW: &str = "desk_config";
const STAFF_VIEW: &str = "staff_desk_config";

const MAIN: &str = "MAIN";
const B9: &str = "B9";

const DATA_FIELDS: [&str; 6] = [
    "branchId",
    "branchName",
    "shiftOpen",
    "adminUrl",
    "rates",
    "paymentMethods",
];
const RATE_FIELDS: [&str; 5] = ["code", "unitSize", "buy", "sell", "publication"];
const METHOD_FIELDS: [&str; 6] = ["id", "enabled", "buy", "sell", "account", "instruction"];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

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

/// A published rate on the wire: a decimal string (W7), read exactly, never through a float.
/// No number of places is required (D-12).
fn rate_text(value: &Value, what: &str) -> Decimal {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{what}: a decimal string on the wire (W7), got {value}"));
    assert!(
        !text.contains(['e', 'E']) && !text.starts_with('+') && text.trim() == text,
        "{what}: a plain decimal string, got {text:?}"
    );
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{what}: {text:?}: {e}"))
}

/// The server's own figure for a rate, whatever its wire form today (v2 prints a JSON number).
fn server_number(value: &Value, what: &str) -> Decimal {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => panic!("{what}: a rate, got {other}"),
    };
    Decimal::from_str(&text).unwrap_or_else(|e| panic!("{what}: {text}: {e}"))
}

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
        let t0 = snapshot_common::time("2026-10-09T05:00:00.000Z");
        desk.clock.set(t0);
        Room {
            desk,
            app,
            cookies,
            t0,
        }
    }

    fn tick(&self) {
        self.desk
            .clock
            .set(self.t0 + Duration::seconds(next() as i64));
    }

    fn cookie(&self, who: &str) -> String {
        match who.split_once('+') {
            Some((a, b)) => format!("{}; {}", self.cookies[a], self.cookies[b]),
            None => self.cookies[who].clone(),
        }
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    /// A view as `who`: 200 with JSON, never cached.
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

    async fn rpc(&self, who: &str, action: &str, payload: Value) -> Answer {
        self.tick();
        let call = Call::action(ORIGIN, RPC, action, payload).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    /// v2's `adminDeskConfig` for `branch`, as the owner: what the view replaces.
    async fn desk_config_v2(&self, branch: &str) -> Value {
        let answer = self
            .rpc("OWNER", "adminDeskConfig", json!({ "branchId": branch }))
            .await;
        assert_eq!(answer.status, 200, "adminDeskConfig: {:.300}", answer.text);
        answer.json()
    }

    /// The rates the server holds for `branch`, THB excluded, in its order. See the file header:
    /// re-pointed at the published rates when #548 lands.
    async fn server_rates(&self, branch: &str) -> Vec<Value> {
        self.desk_config_v2(branch).await["rates"]
            .as_array()
            .expect("adminDeskConfig's rates")
            .clone()
    }

    /// Puts a rate in force for `code` in `branch`. See the file header: v2's direct save, then
    /// the publication of what it saved.
    ///
    /// Amended for #757 (RE-4c, `docs/design/blueprint/rates.md` D-3, §4.9 R10): the rate in
    /// force is the branch's current publication, which `adminDeskConfig` now reads and a direct
    /// save no longer changes; so the branch's newest saved rates are published through the
    /// rate engine (`rates_re4c_common::publish_newest_rates`) after the save, and
    /// `previousUpdated` is read from `rateHistory`.
    async fn publish(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        let snapshot = self
            .rpc("OWNER", "adminSnapshot", json!({ "branchId": branch }))
            .await
            .json();
        // the newest saved row of `code`, which the snapshot lists first in `rateHistory`:
        // `rates[].updated` is the publication's time now
        let previous = snapshot["rateHistory"]
            .as_array()
            .and_then(|rates| rates.iter().find(|r| r["code"] == json!(code)))
            .and_then(|r| r["updated"].as_str())
            .unwrap_or("")
            .to_owned();
        let answer = self
            .rpc(
                "OWNER",
                "adminSaveRate",
                json!({ "branchId": branch, "requestId": request_id(), "code": code,
                        "buy": buy, "sell": sell, "partnerName": "", "partnerCost": null,
                        "previousUpdated": previous }),
            )
            .await;
        assert_eq!(
            answer.status, 200,
            "rate {code} {buy}/{sell} in {branch}: {:.300}",
            answer.text
        );
        snapshot_common::rates_re4c_common::publish_newest_rates(&self.desk.db.pool, branch).await;
    }

    async fn open_shift(&self, branch: &str) {
        let answer = self
            .rpc(
                "OWNER",
                "adminOpenShift",
                json!({ "branchId": branch, "requestId": request_id(),
                        "opening": { "THB": 100000, "USD": 1000 } }),
            )
            .await;
        assert_eq!(
            answer.status, 200,
            "adminOpenShift {branch}: {:.300}",
            answer.text
        );
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// Every caller key of [`Room`] and the actor `Policy` sees for it on `path`: on `StaffRpc` the
/// staff session only, elsewhere the owner first (`rpc.rs`, `view.rs:339-341`).
fn callers(path: EntryPath) -> Vec<(String, Option<Actor>)> {
    let owner = || Some(Actor::owner("owner"));
    let mut out: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        (
            "OWNER".to_string(),
            if path == EntryPath::StaffRpc {
                None
            } else {
                owner()
            },
        ),
    ];
    for role in Role::STAFF {
        let staff = Actor::staff("employee", role, BranchId::new(MAIN));
        out.push((role.as_str().to_string(), staff.clone()));
        out.push((
            format!("OWNER+{}", role.as_str()),
            if path == EntryPath::StaffRpc {
                staff
            } else {
                owner()
            },
        ));
    }
    out
}

fn rate_of<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["rates"]
        .as_array()
        .unwrap_or_else(|| panic!("rates is a list: {data}"))
        .iter()
        .find(|r| r["code"] == json!(code))
        .unwrap_or_else(|| panic!("{code} in rates: {data}"))
}

// ── who may read ─────────────────────────────────────────────────────────────

/// OR-4b (#767, D-4, design §2 and §02 "Server ตรวจ … branch scope"): an employee who names a
/// branch outside their scope is refused 403 `outOfScope`, where v3 answered their own branch.
fn assert_out_of_scope(answer: &rpc_common::Answer, label: &str) {
    assert_eq!(answer.status, 403, "{label}: {:.300}", answer.text);
    assert_eq!(
        answer.json()["error"]["code"],
        serde_json::json!("outOfScope"),
        "{label}: {:.300}",
        answer.text
    );
}

/// DK-A §10: each view refuses exactly whom `Policy::allow` refuses for its pair, caller by
/// caller, with the policy's status and sentence.
#[actix_web::test]
async fn each_view_answers_exactly_whom_its_pair_allows() {
    let room = Room::open().await;
    for (view, path) in [
        (OWNER_VIEW, EntryPath::Rpc),
        (STAFF_VIEW, EntryPath::StaffRpc),
    ] {
        let mut allowed = 0;
        for (who, actor) in callers(path) {
            let answer = room.get(&who, &view_path(view, "")).await;
            let what = format!("{view} as {who:?}");
            match Policy::allow(actor.as_ref(), path, &Action::new("adminDeskConfig")) {
                Ok(()) => {
                    allowed += 1;
                    assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                    assert!(answer.json().get("data").is_some(), "{what}: data");
                }
                Err(e) => {
                    assert_eq!(answer.status, e.status(), "{what}: {:.300}", answer.text);
                    let body = answer.json();
                    assert_eq!(body["error"]["message"], json!(e.to_string()), "{what}");
                    assert!(body.get("data").is_none(), "{what}: no data");
                }
            }
        }
        if view == STAFF_VIEW {
            assert_eq!(allowed, 2, "{view}: FRONT, alone and beside the owner");
        } else {
            assert!(allowed >= 2, "{view}: allowed {allowed}");
        }
    }
    room.finish().await;
}

/// The owner reads the branch named; a FRONT employee on the staff view gets their own branch
/// whatever they name (`Policy::branch`).
#[actix_web::test]
async fn the_owner_reads_the_branch_named_and_front_their_own() {
    let room = Room::open().await;
    let owner = room
        .data("OWNER", OWNER_VIEW, &format!("branch={B9}"))
        .await;
    assert_eq!(owner["branchId"], json!(B9));
    assert_eq!(owner["branchName"], json!("Nine"));
    // OR-4b (D-4): FRONT naming B9 is refused; naming MAIN they read their own
    assert_out_of_scope(
        &room
            .get("FRONT", &view_path(STAFF_VIEW, &format!("branch={B9}")))
            .await,
        "FRONT naming B9",
    );
    let front = room
        .data("FRONT", STAFF_VIEW, &format!("branch={MAIN}"))
        .await;
    assert_eq!(
        front["branchId"],
        json!(MAIN),
        "staff read their own branch"
    );
    room.finish().await;
}

// ── what it answers ──────────────────────────────────────────────────────────

/// §4.1: exactly the six fields; no custody log, no shift id; both paths answer the same data.
#[actix_web::test]
async fn the_data_is_the_six_fields_without_the_custody_log_on_both_paths() {
    let room = Room::open().await;
    room.publish(MAIN, "USD", "35.1", "35.9").await;
    let owner = room
        .data("OWNER", OWNER_VIEW, &format!("branch={MAIN}"))
        .await;
    assert_eq!(keys(&owner), set_of(&DATA_FIELDS), "{owner}");
    for absent in ["operations", "activeShift"] {
        assert!(owner.get(absent).is_none(), "no {absent}: {owner}");
    }
    let usd = rate_of(&owner, "USD");
    assert_eq!(keys(usd), set_of(&RATE_FIELDS), "{usd}");
    for method in owner["paymentMethods"]
        .as_array()
        .expect("paymentMethods is a list")
    {
        assert_eq!(keys(method), set_of(&METHOD_FIELDS), "{method}");
    }
    let front = room.data("FRONT", STAFF_VIEW, "").await;
    assert_eq!(front, owner, "one core, two paths (D-11)");
    room.finish().await;
}

/// §4.1, D-12: the same currencies in the same order as the server holds, THB excluded; each
/// side the server's rate as a decimal string, or null; `unitSize` and `publication` filled.
#[actix_web::test]
async fn rates_are_the_servers_published_rates_as_decimal_strings() {
    let room = Room::open().await;
    room.publish(MAIN, "USD", "35.1234", "35.9").await;
    room.publish(MAIN, "EUR", "38.5", "39.25").await;
    let data = room
        .data("OWNER", OWNER_VIEW, &format!("branch={MAIN}"))
        .await;
    let ours: Vec<&Value> = data["rates"].as_array().expect("rates").iter().collect();
    let theirs = room.server_rates(MAIN).await;
    let codes = |rows: &[&Value]| rows.iter().map(|r| r["code"].clone()).collect::<Vec<_>>();
    assert_eq!(
        codes(&ours),
        codes(&theirs.iter().collect::<Vec<_>>()),
        "same currencies, same order"
    );
    assert!(
        ours.iter().all(|r| r["code"] != json!("THB")),
        "THB is not listed"
    );
    for (mine, server) in ours.iter().zip(theirs.iter()) {
        let code = mine["code"].as_str().unwrap_or("?").to_owned();
        for side in ["buy", "sell"] {
            let what = format!("{code}.{side}");
            if server[side].is_null() {
                assert!(
                    mine[side].is_null(),
                    "{what}: null where the server has none"
                );
            } else {
                assert_eq!(
                    rate_text(&mine[side], &what),
                    server_number(&server[side], &what),
                    "{what}: the server's rate, unchanged"
                );
                assert!(!mine["unitSize"].is_null(), "{what}: unitSize (#547)");
                assert!(!mine["publication"].is_null(), "{what}: publication (#548)");
            }
        }
    }
    assert_eq!(
        rate_text(&rate_of(&data, "USD")["buy"], "USD.buy"),
        Decimal::from_str("35.1234").expect("decimal")
    );
    room.finish().await;
}

/// v2's `activeShift` becomes `shiftOpen`; `adminUrl`, `branchName` and `paymentMethods` are
/// v2's.
#[actix_web::test]
async fn shift_open_follows_the_shift_and_the_rest_is_v2s() {
    let room = Room::open().await;
    let before = room
        .data("OWNER", OWNER_VIEW, &format!("branch={MAIN}"))
        .await;
    assert_eq!(before["shiftOpen"], json!(false), "no shift yet");
    room.open_shift(MAIN).await;
    let after = room
        .data("OWNER", OWNER_VIEW, &format!("branch={MAIN}"))
        .await;
    assert_eq!(after["shiftOpen"], json!(true), "the shift is open");
    let v2 = room.desk_config_v2(MAIN).await;
    assert!(!v2["activeShift"].is_null(), "v2 sees the same open shift");
    assert_eq!(after["adminUrl"], v2["adminUrl"]);
    assert_eq!(after["branchName"], v2["branchName"]);
    assert_eq!(
        after["paymentMethods"], v2["paymentMethods"],
        "v2's rules, in v2's order"
    );
    room.finish().await;
}

// ── the version ──────────────────────────────────────────────────────────────

/// A rate put in force and a shift opened each move the version the view answers; the same
/// `since` then answers `changed: true`, and the current version answers `changed: false`.
#[actix_web::test]
async fn a_new_rate_or_an_opened_shift_moves_the_version() {
    let room = Room::open().await;
    let v0 = room.version(MAIN).await;
    let body = room
        .view("OWNER", OWNER_VIEW, &format!("branch={MAIN}&since={v0}"))
        .await;
    assert_eq!(body["changed"], json!(false), "nothing written yet: {body}");

    room.publish(MAIN, "USD", "35.1", "35.9").await;
    let v1 = room.version(MAIN).await;
    assert!(v1 > v0, "a rate moves the version");
    let body = room
        .view("OWNER", OWNER_VIEW, &format!("branch={MAIN}&since={v0}"))
        .await;
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(body["version"], json!(v1));

    room.open_shift(MAIN).await;
    let v2 = room.version(MAIN).await;
    assert!(v2 > v1, "a shift moves the version");
    let body = room.view("FRONT", STAFF_VIEW, &format!("since={v1}")).await;
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(body["data"]["shiftOpen"], json!(true));
    room.finish().await;
}

// ── the generated table and types ────────────────────────────────────────────

#[test]
fn both_views_are_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    for view in [OWNER_VIEW, STAFF_VIEW] {
        assert!(names.contains(&view), "{view} in {names:?}");
    }
}

/// §6 "Generated types and the two paths": both views write one file, `desk_config.ts`, and it
/// carries the rates as strings (W7).
#[test]
fn both_views_write_desk_config_ts_with_rates_as_strings() {
    for view in [OWNER_VIEW, STAFF_VIEW] {
        // a fresh directory per view: ts-rs skips a path it already wrote in this process
        let dir = std::env::temp_dir().join(format!(
            "xc-desk-config-{view}-{}-{}",
            std::process::id(),
            next()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let entry = VIEWS
            .iter()
            .find(|v| v.name == view)
            .unwrap_or_else(|| panic!("{view} in the table"));
        (entry.export_types)(&dir).unwrap_or_else(|e| panic!("export {view}: {e}"));
        let ts = std::fs::read_to_string(dir.join("desk_config.ts"))
            .unwrap_or_else(|e| panic!("{view} writes desk_config.ts: {e}"));
        assert!(
            !dir.join("staff_desk_config.ts").exists(),
            "{view}: no file of its own for the staff path"
        );
        for field in [
            "branchId",
            "branchName",
            "shiftOpen",
            "adminUrl",
            "rates",
            "paymentMethods",
        ] {
            assert!(
                ts.contains(&format!("{field}: ")),
                "{view}: {field} in {ts}"
            );
        }
        for field in ["unitSize", "publication"] {
            assert!(
                ts.contains(&format!("{field}: ")),
                "{view}: {field} in {ts}"
            );
        }
        assert!(
            ts.contains("buy: Rate | null") || ts.contains("buy: string | null"),
            "{view}: a side is a decimal string or null: {ts}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
