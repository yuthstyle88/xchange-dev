//! Ticket #470 (V2-payments, `docs/design/xchange-v3-web.md` §4.2, §4.4, §5.1, §12.3 in PR 474):
//! the view the admin page's payments menu reads. Written before the implementation by the lane
//! controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `GET /api/v3/views/admin_payments` — a `BranchView`, `INHERITS = (EntryPath::PaymentMethods,
//! "list")`, `NoFilter`, versioned. `data` is the `list` answer of `POST /api/payment-methods`
//! (design `xchange-v3.md` §5.5.2, `PaymentSettingsService::view`) for the same caller and the
//! same branch, with two changes and nothing else:
//!
//! - the setting's own `version` (the 409 guard of row 16) is named `settingsVersion`, so it is
//!   never read as the view's `version` (ticket §7 [required]);
//! - the history entries' keys are camelCase (`createdAt`, `beforeMethods`, `afterMethods`, design
//!   §4.2 [required]: a view's JSON is camelCase); their values are the list's.
//!
//! So `data` is exactly `{branchId, settingsVersion, methods, updatedAt, updatedBy, canEdit,
//! history}`: five rows in v2's order, `history` ≤ 20 newest first and only for the owner
//! (bounded, design §4.2). Staff read their own branch whatever they name (`Policy::branch`).
//! Its one writer, `PaymentSettingsService::save` (the `save` of `/api/payment-methods`), runs
//! inside `BranchTx` (`payment_settings.rs:134-152`), so an equal `since` answers
//! `changed: false` until a save is made on that branch (design §4.2, ticket §7).
//!
//! The view goes through the server's whole route table with real sessions made at the real
//! sign-in doors (`rpc_common`). Saves are made through `/api/payment-methods` itself, each at its
//! own clock time. Nothing is ordered by a timestamp: history is newest first by `version`.
//! Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use compat_common::Person;
use pos_infra::transactions::Transactor;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use snapshot_common::time;
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::test_callers;

const ORIGIN: &str = "http://desk.test";
const ROUTE: &str = "/api/payment-methods";
const VIEW: &str = "admin_payments";

const MAIN: &str = "MAIN";
const AIRPORT: &str = "AIRPORT";
const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const STALE: &str = "มีผู้แก้ไขการตั้งค่านี้แล้ว กรุณาโหลดล่าสุดก่อนบันทึก";
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";

/// The five channels in v2's order (`admin-payments.js:7`, §5.5.2).
const IDS: [&str; 5] = [
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

fn path(query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{VIEW}")
    } else {
        format!("/api/v3/views/{VIEW}?{query}")
    }
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

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// What the view must answer as `data`, from the `list` answer of `/api/payment-methods` for the
/// same caller and branch: `version` renamed `settingsVersion`, history keys camelCase.
fn expected_data(list: &Value) -> Value {
    let history: Vec<Value> = list["history"]
        .as_array()
        .unwrap_or_else(|| panic!("the list answers a history array: {list}"))
        .iter()
        .map(|h| {
            json!({
                "version": h["version"],
                "actor": h["actor"],
                "createdAt": h["created_at"],
                "beforeMethods": h["before_methods"],
                "afterMethods": h["after_methods"],
            })
        })
        .collect();
    json!({
        "branchId": list["branchId"],
        "settingsVersion": list["version"],
        "methods": list["methods"],
        "updatedAt": list["updatedAt"],
        "updatedBy": list["updatedBy"],
        "canEdit": list["canEdit"],
        "history": history,
    })
}

/// Five rows the server accepts: every channel on, SELL everywhere, BUY on Cash and Bank Transfer,
/// each text marked with `tag` so two saves never write the same rows.
fn rows(tag: &str) -> Value {
    let rows: Vec<Value> = IDS
        .iter()
        .map(|id| {
            let buy = *id == "Cash" || *id == "Bank Transfer";
            json!({
                "id": id,
                "enabled": true,
                "buy": buy,
                "sell": true,
                "account": format!("{id} account {tag}"),
                "instruction": format!("{id} instruction {tag}"),
            })
        })
        .collect();
    Value::Array(rows)
}

/// `rows` with one channel switched off.
fn rows_without(tag: &str, off: &str) -> Value {
    let mut rows = rows(tag);
    for row in rows.as_array_mut().expect("five rows") {
        if row["id"] == json!(off) {
            row["enabled"] = json!(false);
        }
    }
    rows
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` (from the migration) and `AIRPORT`; the owner and one employee of every staff role
    /// in `MAIN`, and a Front of `AIRPORT` (key `AIRPORT_FRONT`), all signed in.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(AIRPORT, "Airport").await;
        desk.clock.set(time("2026-10-08T01:00:00Z"));
        let app = rpc_app!(desk);
        let mut people = one_of_each_role();
        people.push(Person {
            key: "AIRPORT_FRONT".to_string(),
            kind: "staff".to_string(),
            id: "00000000-0000-4000-8000-0000000000a1".to_string(),
            username: "front.airport".to_string(),
            name: "Front of the airport".to_string(),
            role: "FRONT".to_string(),
            branch_id: Some(AIRPORT.to_string()),
            must_change: false,
        });
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
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

    /// The view as `who`: 200, JSON, never cached.
    async fn view(&self, who: &str, query: &str) -> Value {
        let p = path(query);
        let answer = self.get(who, &p).await;
        assert_eq!(
            answer.status, 200,
            "GET {p} as {who:?}: {:.400}",
            answer.text
        );
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "GET {p}: never cached"
        );
        answer.json()
    }

    /// `/api/payment-methods` as `who`, answered as it answers.
    async fn send(&self, who: &str, action: &str, payload: Value) -> Answer {
        let call = Call::action(ORIGIN, ROUTE, action, payload).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    /// The `list` answer of `/api/payment-methods` as `who`, naming `branch` (or nothing).
    async fn list(&self, who: &str, branch: Option<&str>) -> Value {
        let payload = branch.map_or_else(|| json!({}), |b| json!({ "branchId": b }));
        let answer = self.send(who, "list", payload).await;
        assert_eq!(answer.status, 200, "list as {who}: {:.400}", answer.text);
        answer.json()
    }

    /// A save by the owner at the next clock minute, which must succeed.
    async fn save(&self, branch: &str, version: i64, methods: &Value) -> Value {
        self.desk.clock.advance(chrono::Duration::minutes(1));
        let answer = self
            .send(
                "OWNER",
                "save",
                json!({ "branchId": branch, "version": version, "methods": methods }),
            )
            .await;
        assert_eq!(
            answer.status, 200,
            "save on {branch} at {version}: {:.400}",
            answer.text
        );
        answer.json()
    }

    /// The setting's own version as `/api/payment-methods` reads it.
    async fn settings_version(&self, branch: &str) -> i64 {
        self.list("OWNER", Some(branch)).await["version"]
            .as_i64()
            .expect("the list answers a whole version")
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    /// One committed `BranchTx` write on `branch` that touches no payment setting: it moves the
    /// branch's version and nothing else.
    async fn bump(&self, branch: &str) {
        BranchTx::new(Transactor::new(self.desk.db.pool.clone()))
            .run(&BranchId::new(branch), |_tx| Box::pin(async { Ok(()) }))
            .await
            .expect("an empty BranchTx write");
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

/// Every caller kind: nobody, the owner, each staff role (of `MAIN`), and the owner with each
/// staff role in the same browser.
fn callers() -> Vec<(String, Option<Actor>)> {
    let mut callers: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        ("OWNER".to_string(), Some(test_callers::owner("owner"))),
    ];
    for (name, roles) in STAFF {
        callers.push((
            name.to_string(),
            Some(test_callers::staff("employee", roles, BranchId::new(MAIN))),
        ));
        callers.push((format!("OWNER+{name}"), Some(test_callers::owner("owner"))));
    }
    callers
}

// ── who may read (W8, design §4.4) ───────────────────────────────────────────

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

/// OR-4b (#767): the rule a route asks once it is on `Policy::authorize` (design §4.2): the caller
/// as B1 maps them (§3.3), naming `named` (D-4: a branch outside the caller's scope is refused).
fn blueprint_rule(
    actor: Option<&Actor>,
    path: EntryPath,
    action: &str,
    named: Option<&str>,
) -> Result<(), xc_auth::domain::policy::value_objects::policy_error::PolicyError> {
    use xc_auth::domain::policy::entities::policy::Policy;
    let act = xc_auth::domain::policy::value_objects::action::Action::new(action);
    Policy::authorize(
        actor,
        path,
        &act,
        named.map(xc_kernel::ids::BranchId::new).as_ref(),
        chrono::Utc::now(),
    )
    .map(|_| ())
}

#[actix_web::test]
async fn admin_payments_asks_nobody_signed_in_to_sign_in() {
    let room = Room::open().await;
    let answer = room.get("", &path("branch=MAIN")).await;
    assert_eq!(answer.status, 401, "{:.300}", answer.text);
    assert_eq!(
        answer.json(),
        json!({ "error": { "code": "signIn", "message": SIGN_IN } })
    );
    room.finish().await;
}

/// The view answers exactly whom `Policy::authorize(_, PaymentMethods, "list", ..)` answers (every
/// signed-in caller), and each caller gets the list answer `/api/payment-methods` gives the same
/// cookies for the same named branch: the owner the named branch with `canEdit: true` and the
/// history, staff their own branch with `canEdit: false` and no history.
#[actix_web::test]
async fn admin_payments_answers_every_caller_what_the_payment_methods_list_answers_them() {
    let room = Room::open().await;
    let v = room.settings_version(AIRPORT).await;
    room.save(AIRPORT, v, &rows("airport")).await;
    let mut allowed = 0;
    for (who, actor) in callers() {
        // OR-4b (#767): the route table's rule, naming AIRPORT (D-4: an employee of MAIN is refused)
        let policy = blueprint_rule(
            actor.as_ref(),
            EntryPath::PaymentMethods,
            "list",
            Some(AIRPORT),
        );
        let answer = room.get(&who, &path("branch=AIRPORT")).await;
        let what = format!("{VIEW} as {who:?}");
        match &policy {
            Ok(()) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["changed"], json!(true), "{what}: {body}");
                let list = room.list(&who, Some(AIRPORT)).await;
                assert_eq!(
                    body["data"],
                    expected_data(&list),
                    "{what}: the list answer of the same caller"
                );
                let owner = actor
                    .as_ref()
                    .is_some_and(|a| a.roles.iter().all(|r| r.branch.is_none()));
                assert_eq!(body["data"]["canEdit"], json!(owner), "{what}");
                if owner {
                    assert_eq!(body["data"]["branchId"], json!(AIRPORT), "{what}");
                    assert_eq!(
                        body["data"]["history"].as_array().map(Vec::len),
                        Some(1),
                        "{what}: the owner sees the history"
                    );
                } else {
                    assert_eq!(body["data"]["branchId"], json!(MAIN), "{what}: their own");
                    assert_eq!(body["data"]["history"], json!([]), "{what}: no history");
                }
            }
            Err(e) => {
                assert_eq!(answer.status, e.status(), "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["error"]["message"], json!(e.to_string()), "{what}");
                assert!(body.get("data").is_none(), "{what}: no data");
            }
        }
    }
    // D-4: an employee of MAIN naming AIRPORT is refused; the owner, alone or not, reads it
    assert_eq!(
        allowed,
        1 + STAFF.len(),
        "the owner, alone or beside an employee, reads AIRPORT"
    );
    room.finish().await;
}

// ── what it answers (ticket §7, §8) ──────────────────────────────────────────

/// Before any save: v2's defaults in v2's order, `settingsVersion` 0, never edited, no history;
/// `data` has exactly the seven keys of ticket §7 and the answer exactly `{version, changed,
/// data}`, `version` being the branch's.
#[actix_web::test]
async fn admin_payments_answers_the_defaults_before_any_save() {
    let room = Room::open().await;
    let body = room.view("OWNER", "branch=MAIN").await;
    assert_eq!(keys(&body), set(&["changed", "data", "version"]), "{body}");
    assert_eq!(body["changed"], json!(true));
    assert_eq!(body["version"], json!(room.version(MAIN).await), "{body}");
    let data = &body["data"];
    assert_eq!(
        keys(data),
        set(&[
            "branchId",
            "settingsVersion",
            "methods",
            "updatedAt",
            "updatedBy",
            "canEdit",
            "history"
        ]),
        "exactly the list answer's keys, `version` named `settingsVersion`: {data}"
    );
    assert_eq!(data["branchId"], json!(MAIN));
    assert_eq!(data["settingsVersion"], json!(0));
    assert_eq!(data["updatedAt"], Value::Null);
    assert_eq!(data["updatedBy"], Value::Null);
    assert_eq!(data["canEdit"], json!(true));
    assert_eq!(data["history"], json!([]));
    let methods = data["methods"].as_array().expect("methods");
    assert_eq!(
        methods.iter().map(|m| m["id"].clone()).collect::<Vec<_>>(),
        IDS.iter().map(|id| json!(id)).collect::<Vec<_>>(),
        "five rows in v2's order"
    );
    for row in methods {
        assert_eq!(
            keys(row),
            set(&["id", "enabled", "buy", "sell", "account", "instruction"]),
            "{row}"
        );
    }
    assert_eq!(
        data.clone(),
        expected_data(&room.list("OWNER", Some(MAIN)).await),
        "the list's defaults"
    );
    room.finish().await;
}

/// The owner reads the branch they name (no name = `MAIN`); an employee reads their own branch
/// whatever they name, without its history.
#[actix_web::test]
async fn admin_payments_answers_the_named_branch_for_the_owner_and_their_own_for_staff() {
    let room = Room::open().await;
    let owner_id: String = sqlx::query_scalar(
        "SELECT u.id::text FROM users u \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("read owner users.id");
    room.save(AIRPORT, 0, &rows_without("airport", "Alipay"))
        .await;
    room.save(MAIN, 0, &rows("main")).await;

    let airport = room.view("OWNER", "branch=AIRPORT").await;
    assert_eq!(airport["data"]["branchId"], json!(AIRPORT));
    assert_eq!(
        airport["data"]["methods"],
        rows_without("airport", "Alipay")
    );
    // OR-7.m G2: rule 2 (#3700)
    assert_eq!(airport["data"]["updatedBy"], json!(owner_id));
    assert_eq!(
        airport["data"],
        expected_data(&room.list("OWNER", Some(AIRPORT)).await)
    );

    for query in ["", "branch=MAIN"] {
        let main = room.view("OWNER", query).await;
        assert_eq!(main["data"]["branchId"], json!(MAIN), "{query:?}");
        assert_eq!(main["data"]["methods"], rows("main"), "{query:?}");
    }

    // OR-4b (D-4): staff of AIRPORT naming MAIN are refused; naming none or AIRPORT they read
    // AIRPORT's
    assert_out_of_scope(
        &room.get("AIRPORT_FRONT", &path("branch=MAIN")).await,
        "MAIN",
    );
    for query in ["", "branch=AIRPORT"] {
        let staff = room.view("AIRPORT_FRONT", query).await;
        assert_eq!(staff["data"]["branchId"], json!(AIRPORT), "{query:?}");
        assert_eq!(
            staff["data"]["methods"],
            rows_without("airport", "Alipay"),
            "{query:?}"
        );
        assert_eq!(staff["data"]["canEdit"], json!(false), "{query:?}");
        assert_eq!(staff["data"]["history"], json!([]), "{query:?}");
        assert_eq!(
            staff["version"],
            json!(room.version(AIRPORT).await),
            "{query:?}: AIRPORT's version"
        );
        assert_eq!(
            staff["data"],
            expected_data(&room.list("AIRPORT_FRONT", Some(AIRPORT)).await),
            "{query:?}"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn admin_payments_refuses_an_unknown_branch() {
    let room = Room::open().await;
    let answer = room.get("OWNER", &path("branch=NOPE")).await;
    assert_eq!(answer.status, 400, "{:.300}", answer.text);
    assert_eq!(
        answer.json(),
        json!({ "error": { "code": "noSuchBranch", "field": "branch", "message": NO_SUCH_BRANCH } })
    );
    room.finish().await;
}

/// `settingsVersion` is the setting's own version, the one `save` checks (row 16), and not the
/// branch's version the view answers as `version`: the two are made to differ, a save sent with
/// `settingsVersion` succeeds, and one sent with the view's `version` is refused 409.
#[actix_web::test]
async fn admin_payments_settings_version_is_the_409_guard_not_the_views_version() {
    let room = Room::open().await;
    room.save(MAIN, 0, &rows("one")).await;
    for _ in 0..3 {
        room.bump(MAIN).await;
    }
    let body = room.view("OWNER", "branch=MAIN").await;
    let view_version = body["version"].as_i64().expect("a version");
    let settings_version = body["data"]["settingsVersion"]
        .as_i64()
        .expect("a settingsVersion");
    assert_eq!(settings_version, room.settings_version(MAIN).await);
    assert_eq!(settings_version, 1, "one save");
    assert_eq!(view_version, room.version(MAIN).await);
    assert_ne!(
        view_version, settings_version,
        "the branch moved without the setting"
    );

    let refused = room
        .send(
            "OWNER",
            "save",
            json!({ "branchId": MAIN, "version": view_version, "methods": rows("two") }),
        )
        .await;
    assert_eq!(refused.status, 409, "{:.300}", refused.text);
    assert_eq!(refused.json(), json!({ "error": STALE }));

    let saved = room.save(MAIN, settings_version, &rows("two")).await;
    assert_eq!(saved["version"], json!(2));
    let after = room.view("OWNER", "branch=MAIN").await;
    assert_eq!(after["data"]["settingsVersion"], json!(2));
    assert_eq!(after["data"]["methods"], rows("two"));
    room.finish().await;
}

/// Bounded (design §4.2): after 22 saves the owner reads the last 20, newest first by version,
/// each the list's entry with camelCase keys; `before`/`after` are the rows before and after that
/// save. Staff of the same branch read none.
#[actix_web::test]
async fn admin_payments_history_is_the_last_twenty_newest_first_for_the_owner_only() {
    let room = Room::open().await;
    let owner_id: String = sqlx::query_scalar(
        "SELECT u.id::text FROM users u \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("read owner users.id");
    for n in 0..22 {
        room.save(MAIN, n, &rows(&format!("n{n}"))).await;
    }
    let body = room.view("OWNER", "branch=MAIN").await;
    let history = body["data"]["history"].as_array().expect("history");
    assert_eq!(history.len(), 20, "at most 20");
    assert_eq!(
        history
            .iter()
            .map(|h| h["version"].clone())
            .collect::<Vec<_>>(),
        (3..=22).rev().map(|v| json!(v)).collect::<Vec<_>>(),
        "newest first"
    );
    for h in history {
        assert_eq!(
            keys(h),
            set(&[
                "version",
                "actor",
                "createdAt",
                "beforeMethods",
                "afterMethods"
            ]),
            "{h}"
        );
        // OR-7.m G2: rule 2 (#3700)
        assert_eq!(h["actor"], json!(owner_id), "{h}");
        let v = h["version"].as_i64().expect("a version");
        assert_eq!(h["afterMethods"], rows(&format!("n{}", v - 1)), "{h}");
        assert_eq!(h["beforeMethods"], rows(&format!("n{}", v - 2)), "{h}");
    }
    assert_eq!(
        body["data"],
        expected_data(&room.list("OWNER", Some(MAIN)).await)
    );
    let staff = room.view("FRONT", "branch=MAIN").await;
    assert_eq!(staff["data"]["history"], json!([]));
    assert_eq!(staff["data"]["settingsVersion"], json!(22));
    room.finish().await;
}

/// The first save's `before` is v2's defaults, as the list answers it.
#[actix_web::test]
async fn admin_payments_first_history_entry_starts_from_the_defaults() {
    let room = Room::open().await;
    let defaults = room.view("OWNER", "branch=MAIN").await["data"]["methods"].clone();
    room.save(MAIN, 0, &rows("first")).await;
    let body = room.view("OWNER", "branch=MAIN").await;
    let history = body["data"]["history"].as_array().expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["version"], json!(1));
    assert_eq!(history[0]["beforeMethods"], defaults);
    assert_eq!(history[0]["afterMethods"], rows("first"));
    assert_eq!(
        body["data"]["updatedAt"], history[0]["createdAt"],
        "the save's own time"
    );
    room.finish().await;
}

/// Versioned, with `PaymentSettingsService::save` as the writer (design §4.2, ticket §7): an
/// equal `since` answers `changed: false` with no data; one save moves the branch's version, so
/// the same `since` then answers `changed: true` with what was saved; a save on another branch
/// leaves this branch's `since` unchanged.
#[actix_web::test]
async fn admin_payments_follows_its_branchs_since_through_the_save() {
    let room = Room::open().await;
    let owner_id: String = sqlx::query_scalar(
        "SELECT u.id::text FROM users u \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("read owner users.id");
    let v0 = room.version(MAIN).await;
    let same = room.view("OWNER", &format!("branch=MAIN&since={v0}")).await;
    assert_eq!(
        same,
        json!({ "version": v0, "changed": false }),
        "nothing changed: O(1), no data"
    );

    room.save(MAIN, 0, &rows("saved")).await;
    let v1 = room.version(MAIN).await;
    assert!(v1 > v0, "the save moves MAIN's version ({v0} → {v1})");
    let after = room.view("OWNER", &format!("branch=MAIN&since={v0}")).await;
    assert_eq!(after["changed"], json!(true), "{after}");
    assert_eq!(after["version"], json!(v1), "{after}");
    assert_eq!(after["data"]["settingsVersion"], json!(1), "{after}");
    assert_eq!(after["data"]["methods"], rows("saved"), "{after}");
    // OR-7.m G2: rule 2 (#3700)
    assert_eq!(after["data"]["updatedBy"], json!(owner_id), "{after}");

    let quiet = room.view("OWNER", &format!("branch=MAIN&since={v1}")).await;
    assert_eq!(quiet, json!({ "version": v1, "changed": false }));

    room.save(AIRPORT, 0, &rows("elsewhere")).await;
    let still = room.view("OWNER", &format!("branch=MAIN&since={v1}")).await;
    assert_eq!(
        still,
        json!({ "version": v1, "changed": false }),
        "another branch's save"
    );
    let staff = room
        .view(
            "AIRPORT_FRONT",
            &format!("since={}", room.version(AIRPORT).await - 1),
        )
        .await;
    assert_eq!(staff["changed"], json!(true), "{staff}");
    assert_eq!(staff["data"]["methods"], rows("elsewhere"), "{staff}");
    room.finish().await;
}

// ── the generated types (§5.1) ───────────────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("xc-v3-payments-{}-{}", std::process::id(), next()));
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

fn exported() -> (String, BTreeSet<String>) {
    let view = VIEWS.iter().find(|v| v.name == VIEW).unwrap_or_else(|| {
        panic!(
            "{VIEW} is in the generated view table: {:?}",
            VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
        )
    });
    let scratch = Scratch::new();
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {VIEW}: {e}"));
    let files = std::fs::read_dir(&scratch.0)
        .expect("read the scratch directory")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let ts = std::fs::read_to_string(scratch.0.join(format!("{VIEW}.ts")))
        .unwrap_or_else(|e| panic!("{VIEW} writes {VIEW}.ts: {e}"));
    (ts, files)
}

#[test]
fn the_payments_type_carries_the_settings_rows_and_the_history() {
    let (ts, files) = exported();
    for file in &files {
        assert!(
            file == "admin_payments.ts" || file == "common.ts",
            "{VIEW} writes only its own file (and common.ts): {files:?}"
        );
    }
    for field in [
        "branchId: string",
        "settingsVersion: number",
        "methods: Array<",
        "id: string",
        "enabled: boolean",
        "buy: boolean",
        "sell: boolean",
        "account: string",
        "instruction: string",
        "updatedAt: string | null",
        "updatedBy: string | null",
        "canEdit: boolean",
        "history: Array<",
        "version: number",
        "actor: string",
        "createdAt: string",
        "beforeMethods: Array<",
        "afterMethods: Array<",
    ] {
        assert!(ts.contains(field), "{field}\n{ts}");
    }
    for never in [
        "bigint",
        "JsonValue",
        "created_at",
        "before_methods",
        ": any",
    ] {
        assert!(!ts.contains(never), "no {never}\n{ts}");
    }
}

#[test]
fn the_generated_file_is_committed() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../web/src/shared/api/generated/{VIEW}.ts"));
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/{VIEW}.ts: {e}"));
    assert_eq!(
        committed,
        exported().0,
        "{VIEW}.ts is what the DTOs generate"
    );
}
