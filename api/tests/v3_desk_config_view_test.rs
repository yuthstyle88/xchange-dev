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
#[macro_use]
mod rpc_common;
mod shift_setup_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::test_callers;

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

    /// Puts a rate in force for `code` in `branch`. See the file header.
    ///
    /// Amended for #757 (RE-4c, `docs/design/blueprint/rates.md` D-3, §4.9 R10): the rate in
    /// force is the branch's current publication. Re-expected for RE-7 #604 by the ticket author:
    /// the direct save is retired (design §9), so the helper is re-pointed at the publication
    /// path alone, through the owner's own routes (`POST /api/rate-drafts`, `…/submit`,
    /// `…/publish`), as a v3 shop sets a rate: the branch's other published lines are kept and
    /// `code` is set at `buy` / `sell` per 1 (× its `unit_size`). The routes, not a service call,
    /// so the write is a write of the branch, as the direct save was. No assertion changes.
    async fn publish(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        let pool = &self.desk.db.pool;
        let kept: Vec<(String, Decimal, Decimal)> = sqlx::query_as(
            "SELECT trim(l.currency_code), l.buy_rate, l.sell_rate \
             FROM branches b JOIN published_rate_lines l \
               ON l.publication_id = b.current_publication_id \
             WHERE b.id = $1 AND l.denomination_group = 'ALL' \
               AND l.buy_rate IS NOT NULL AND l.sell_rate IS NOT NULL ORDER BY 1",
        )
        .bind(branch)
        .fetch_all(pool)
        .await
        .expect("read the branch's current lines");
        let unit: Decimal = sqlx::query_scalar("SELECT unit_size FROM currencies WHERE code = $1")
            .bind(code)
            .fetch_one(pool)
            .await
            .expect("the currency's unit size");
        let per_unit = |per_one: &str| {
            (Decimal::from_str(per_one).expect("a rate") * unit)
                .normalize()
                .to_string()
        };
        let line = |c: &str, b: String, s: String| {
            json!({ "currency": c, "buyMode": "MANUAL", "sellMode": "MANUAL",
                    "manualBuy": b, "manualSell": s, "reason": "set by the desk config test" })
        };
        let mut lines: Vec<Value> = kept
            .iter()
            .filter(|(c, _, _)| c != code)
            .map(|(c, b, s)| line(c, b.normalize().to_string(), s.normalize().to_string()))
            .collect();
        lines.push(line(code, per_unit(buy), per_unit(sell)));
        let expected: Option<i32> = sqlx::query_scalar(
            "SELECT p.version FROM branches b JOIN rate_publications p \
               ON p.id = b.current_publication_id WHERE b.id = $1",
        )
        .bind(branch)
        .fetch_optional(pool)
        .await
        .expect("read the current version");
        let command = |path: String, body: Value| {
            self.tick();
            Call::post(ORIGIN, &path, body.to_string())
                .header("idempotency-key", &request_id())
                .cookie(&self.cookie("OWNER"))
        };
        let draft = self
            .app
            .send(command(
                "/api/rate-drafts".to_owned(),
                json!({ "branch": branch, "lines": lines }),
            ))
            .await;
        assert_eq!(
            draft.status, 200,
            "draft {code} in {branch}: {:.300}",
            draft.text
        );
        let id = draft.json()["id"]
            .as_str()
            .expect("the draft's id")
            .to_owned();
        let submit = self
            .app
            .send(command(
                format!("/api/rate-drafts/{id}/submit"),
                json!({ "branch": branch }),
            ))
            .await;
        assert_eq!(
            submit.status, 200,
            "submit {code} in {branch}: {:.300}",
            submit.text
        );
        let publish = self
            .app
            .send(command(
                format!("/api/rate-drafts/{id}/publish"),
                json!({ "branch": branch, "expectedVersion": expected }),
            ))
            .await;
        assert_eq!(
            publish.status, 200,
            "rate {code} {buy}/{sell} in {branch}: {:.300}",
            publish.text
        );
    }

    async fn open_shift(&self, branch: &str) {
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            branch,
            json!({ "THB": 100000, "USD": 1000 }),
            chrono::Utc::now(),
        )
        .await;
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

const STAFF: [(&str, &[Role]); 9] = [
    ("FRONT", &[Role::Teller]),
    ("BACKEND", &[Role::BranchManager]),
    ("MANAGER", &[Role::BranchManager]),
    ("RATES", &[Role::RateManager]),
    ("MONITOR", &[Role::VerificationReviewer]),
    ("FINANCE", &[Role::Finance]),
    ("SYSTEM", &[Role::Admin]),
    ("DOCUMENTS", &[Role::VerificationReviewer]),
    ("DOCUMENT_REVIEW", &[Role::VerificationReviewer]),
];

/// Every caller key of [`Room`] and the actor `Policy` sees for it, the owner first: on `StaffRpc`
/// too, since the owner is a staff caller there (RBAC §11.8 S10, #4528).
fn callers() -> Vec<(String, Option<Actor>)> {
    let owner = || Some(test_callers::owner("owner"));
    let mut out: Vec<(String, Option<Actor>)> =
        vec![(String::new(), None), ("OWNER".to_string(), owner())];
    for (name, roles) in STAFF {
        let staff = Some(test_callers::staff("employee", roles, BranchId::new(MAIN)));
        out.push((name.to_string(), staff));
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

/// DK-A §10: each view refuses exactly whom `Policy::authorize` / `RoleGrants::seed` refuses for
/// its pair, caller by caller, with the policy's status and sentence.
#[actix_web::test]
async fn each_view_answers_exactly_whom_its_pair_allows() {
    use xc_auth::domain::access::{Permission, RoleGrants};
    use xc_auth::domain::policy::entities::policy::Need;

    let room = Room::open().await;
    let act = Action::new("adminDeskConfig");
    for (view, path) in [
        (OWNER_VIEW, EntryPath::Rpc),
        (STAFF_VIEW, EntryPath::StaffRpc),
    ] {
        let Some(Need::Permission(perm)) = Policy::need(path, &act) else {
            panic!("({path:?}, adminDeskConfig) needs a permission");
        };
        let mut allowed = 0;
        let mut refused = 0;
        for (who, actor) in callers() {
            let answer = room.get(&who, &view_path(view, "")).await;
            let what = format!("{view} as {who:?}");
            // OR-7.m (merge of feat/rbac-groups): the caller is already a Blueprint actor
            let holds = actor.as_ref().is_some_and(|ac| {
                ac.roles
                    .iter()
                    .any(|r| RoleGrants::seed(r.role).contains(&perm))
            });
            let auth = Policy::authorize(actor.as_ref(), path, &act, None, Utc::now());
            assert_eq!(
                auth.is_ok(),
                holds,
                "{what}: RoleGrants::seed matches Policy::authorize"
            );
            match auth {
                Ok(_) => {
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
        if view == STAFF_VIEW {
            // OR-7.m G2: rule 4 (#3700, #3717): one session per request, so each staff role once
            let expected_staff = STAFF
                .iter()
                .filter(|(_, roles)| {
                    roles
                        .iter()
                        .any(|&r| RoleGrants::seed(r).contains(&Permission::ExchangeTransact))
                })
                .count();
            // + 1 is the owner (§11.8 S10, #4528)
            assert_eq!(
                allowed,
                expected_staff + 1,
                "{view}: staff roles with ExchangeTransact plus owner"
            );
            assert!(
                refused > 2,
                "{view}: §11.2 leaves staff roles without ExchangeTransact"
            );
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
