//! Ticket #462 (V2-employees, `docs/design/xchange-v3-web.md` §4.2, §4.4, §12.7 in PR 474): the two
//! views the admin page's employees menu reads. Written before the implementation by the lane
//! controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/admin_employees` — a `BranchView`, `INHERITS = (EntryPath::OwnerRoute,
//!   "list")`, `NoFilter`, **`VERSIONED = false`** (design §12.7): `data.employees` is exactly
//!   the `employees` of `/api/employees` `list` (`employees.rs:197-201`, `employee_json`):
//!   every account of every branch, `{id, username, name, role, branchId, active, mustChange}`,
//!   in the list's order. Staff are refused as `/api/employees` refuses them (W8, ticket §8).
//!   The answer always has data, whatever `since` says; `version` is the named branch's.
//! - `GET /api/v3/views/admin_operations` — a `BranchView`, `INHERITS = (EntryPath::Rpc,
//!   "adminSnapshot")`, `NoFilter`, versioned: `data` is exactly `{enabled, entity}` of the
//!   snapshot's `operations` for the branch (`snapshot.rs:171-176`, `v2_json.rs:489-515`),
//!   `entity` being `null` or carrying the snapshot's `company` and `site`. Nothing that grows
//!   with history (`events`, `requests`, `attendance`) is in it (design §4.2: a view's data is
//!   bounded). Its writers, `/api/employees` `entity` and `enable`, run inside `BranchTx`
//!   (`infrastructure/custody/employee_custody.rs:62-73`), so an equal `since` answers
//!   `changed: false` until one of them is made (design §4.2, ticket §9 row 3).
//!
//! The views go through the server's whole route table with real sessions made at the real
//! sign-in doors (`rpc_common`). Branches are inserted with explicit, distinct `created_at`;
//! nothing is ordered by a frozen clock. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use compat_common::Person;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";
const EMPLOYEES: &str = "/api/employees";
const STAFF_VIEW: &str = "admin_employees";
const OPS_VIEW: &str = "admin_operations";

const MAIN: &str = "MAIN";
const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
/// A temporary password v2's rules accept (12–128 characters).
const PASSWORD: &str = "temporary-pass-01";

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
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

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` (from the migration), then `Z9` and `A1` created after it in that order; the owner
    /// and one employee of every staff role in `MAIN` (so `MAIN` has the six roles `enable`
    /// needs), and a FRONT employee of `A1` (key `FRONT_A1`), all signed in.
    async fn open() -> Room {
        let desk = Desk::open().await;
        for (id, name, at) in [
            ("Z9", "Zulu", "2026-10-07T01:00:00Z"),
            ("A1", "Alpha", "2026-10-07T02:00:00Z"),
        ] {
            sqlx::query(
                "INSERT INTO branches (id, name, created_at) VALUES ($1, $2, $3::timestamptz)",
            )
            .bind(id)
            .bind(name)
            .bind(at)
            .execute(&desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("insert branch {id}: {e}"));
        }
        let app = rpc_app!(desk);
        let mut people = one_of_each_role();
        people.push(Person {
            key: "FRONT_A1".to_string(),
            kind: "staff".to_string(),
            id: "00000000-0000-4000-8000-0000000000a1".to_string(),
            username: "fronta1".to_string(),
            name: "Front of A1".to_string(),
            role: "FRONT".to_string(),
            branch_id: Some("A1".to_string()),
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

    /// `view` as `who`: 200, JSON, never cached.
    async fn view(&self, view: &str, who: &str, query: &str) -> Value {
        let p = path(view, query);
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

    /// `/api/employees` `{action, payload}` as `who`, answered as it answers.
    async fn employees_call(&self, who: &str, action: &str, payload: Value) -> Answer {
        let call = Call::action(ORIGIN, EMPLOYEES, action, payload);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    /// `/api/employees` `{action, payload}` as the owner, which must succeed.
    async fn employees(&self, action: &str, payload: Value) -> Value {
        let answer = self.employees_call("OWNER", action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "{action} with {payload}: {:.400}",
            answer.text
        );
        answer.json()
    }

    /// `branch`'s entity. It has no writer since CA-10 (#626 Q5, Q7): a read-only column, seeded
    /// here by SQL in the shape the custody repository reads (`custody_repository.rs`
    /// `entity_to_json`).
    async fn seed_entity(&self, branch: &str, company: &str, site: &str) {
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase, entity) VALUES ($1, 'CLOSED', \
             jsonb_build_object('company', $2::text, 'site', $3::text, 'confirmedBy', 'owner', \
             'at', 1791342000000::bigint)) \
             ON CONFLICT (branch_id) DO UPDATE SET entity = EXCLUDED.entity",
        )
        .bind(branch)
        .bind(company)
        .bind(site)
        .execute(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("seed {branch}'s entity: {e}"));
    }

    /// The `employees` of `/api/employees` `list`, as the owner reads it.
    async fn list(&self) -> Value {
        let answer = self.employees("list", json!({})).await;
        assert!(
            answer["employees"].is_array(),
            "the list answers employees: {answer:.300}"
        );
        answer["employees"].clone()
    }

    async fn rpc(&self, who: &str, action: &str, payload: Value) -> Value {
        let call = Call::action(ORIGIN, RPC, action, payload.clone()).cookie(&self.cookie(who));
        let answer = self.app.send(call).await;
        assert_eq!(
            answer.status, 200,
            "{action} as {who} with {payload}: {:.400}",
            answer.text
        );
        answer.json()
    }

    /// `operations` of `adminSnapshot` for `branch`, as `who` reads it.
    async fn operations(&self, who: &str, branch: &str) -> Value {
        let snapshot = self
            .rpc(who, "adminSnapshot", json!({ "branchId": branch }))
            .await;
        let operations = snapshot["operations"].clone();
        assert!(
            operations.is_object(),
            "the snapshot has operations: {snapshot:.300}"
        );
        operations
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    /// A new account through `/api/employees` `create`; answers its id from the list.
    async fn create(&self, username: &str, role: &str, branch: &str) -> String {
        self.employees(
            "create",
            json!({ "name": format!("Name of {username}"), "username": username,
                    "role": role, "branchId": branch, "password": PASSWORD }),
        )
        .await;
        let list = self.list().await;
        list.as_array()
            .and_then(|l| l.iter().find(|e| e["username"] == json!(username)))
            .and_then(|e| e["id"].as_str())
            .unwrap_or_else(|| panic!("{username} is listed after create: {list}"))
            .to_string()
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// Every caller kind: nobody, the owner, each staff role (of `MAIN`), and the owner with each
/// staff role in the same browser.
fn callers() -> Vec<(String, Option<Actor>)> {
    let mut callers: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        ("OWNER".to_string(), Some(Actor::owner("owner"))),
    ];
    for role in Role::STAFF {
        callers.push((
            role.as_str().to_string(),
            Actor::staff("employee", role, BranchId::new(MAIN)),
        ));
        callers.push((
            format!("OWNER+{}", role.as_str()),
            Some(Actor::owner("owner")),
        ));
    }
    callers
}

/// The `{entity}` the view must answer, read from the snapshot's `operations`: `entity` is `null`
/// or an object whose `company` and `site` are the snapshot's. Re-expected for CA-10 #626 by the
/// ticket author (ruling Q8 A): `enabled` left the view with the Enable switch (design §6); the
/// compat snapshot keeps it as a constant `false`.
fn assert_operations_match(data: &Value, operations: &Value, what: &str) {
    assert_eq!(
        keys(data),
        set(&["entity"]),
        "{what}: data is exactly {{entity}} (bounded, design §4.2): {data}"
    );
    assert_eq!(
        operations["enabled"],
        json!(false),
        "{what}: the compat snapshot's enabled is a constant false (Q8 A)"
    );
    if operations["entity"].is_null() {
        assert_eq!(data["entity"], Value::Null, "{what}: no entity yet");
    } else {
        assert_eq!(
            data["entity"]["company"], operations["entity"]["company"],
            "{what}: entity.company is the snapshot's"
        );
        assert_eq!(
            data["entity"]["site"], operations["entity"]["site"],
            "{what}: entity.site is the snapshot's"
        );
    }
}

// ── admin_employees: who may read (W8, ticket §8) ───────────────────────────

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
    actor: Option<&xc_auth::domain::policy::value_objects::actor::Actor>,
    path: xc_auth::domain::policy::value_objects::entry_path::EntryPath,
    action: &str,
    named: Option<&str>,
) -> Result<(), xc_auth::domain::policy::value_objects::policy_error::PolicyError> {
    use xc_auth::domain::policy::entities::policy::{Need, Policy};
    let act = xc_auth::domain::policy::value_objects::action::Action::new(action);
    if Policy::need(path, &act) == Some(Need::Custody) {
        // #582's rows stay on the v2 rule
        return Policy::allow(actor, path, &act);
    }
    let user = actor.map(|a| {
        xc_auth::domain::access::v3_roles::actor_from_v3(
            a,
            "XC",
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        )
    });
    Policy::authorize(
        user.as_ref(),
        path,
        &act,
        named.map(xc_kernel::ids::BranchId::new).as_ref(),
        chrono::Utc::now(),
    )
    .map(|_| ())
}

#[actix_web::test]
async fn admin_employees_asks_nobody_signed_in_to_sign_in() {
    let room = Room::open().await;
    let answer = room.get("", &path(STAFF_VIEW, "branch=MAIN")).await;
    assert_eq!(answer.status, 401, "{:.300}", answer.text);
    assert_eq!(
        answer.json(),
        json!({ "error": { "code": "signIn", "message": SIGN_IN } })
    );
    room.finish().await;
}

/// The view answers exactly whom `Policy::allow(_, OwnerRoute, "list")` answers, for every
/// caller kind, and refuses with the policy's status and sentence. A caller alone in the browser
/// gets the same status and sentence from the view as from v2's own door, `/api/employees`
/// `list` (ticket §8: "v2's `/api/employees` refuses staff too").
#[actix_web::test]
async fn admin_employees_answers_exactly_whom_the_owner_route_allows() {
    let room = Room::open().await;
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        // OR-4b (#767): the route table's rule (C4: UsersManage is ADMIN's, so SYSTEM reads it)
        let policy = blueprint_rule(actor.as_ref(), EntryPath::OwnerRoute, "list", Some(MAIN));
        let answer = room.get(&who, &path(STAFF_VIEW, "branch=MAIN")).await;
        let what = format!("{STAFF_VIEW} as {who:?}");
        match &policy {
            Ok(()) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                assert_eq!(answer.json()["changed"], json!(true), "{what}");
                assert!(answer.json()["data"]["employees"].is_array(), "{what}");
            }
            Err(e) => {
                refused += 1;
                assert_eq!(answer.status, e.status(), "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["error"]["message"], json!(e.to_string()), "{what}");
                assert!(body.get("data").is_none(), "{what}: no data");
                if !who.contains('+') {
                    let v2 = room.employees_call(&who, "list", json!({})).await;
                    assert_eq!(
                        answer.status, v2.status,
                        "{what}: the status v2's door gives"
                    );
                    assert_eq!(
                        body["error"]["message"],
                        v2.json()["error"],
                        "{what}: the sentence v2's door gives"
                    );
                }
            }
        }
    }
    // C4: and SYSTEM, which B1 makes ADMIN
    assert_eq!(
        allowed,
        1 + Role::STAFF.len() + 1,
        "the owner, alone or not, and SYSTEM"
    );
    assert_eq!(
        refused,
        1 + Role::STAFF.len() - 1,
        "nobody, and every staff role but SYSTEM"
    );
    room.finish().await;
}

// ── admin_employees: what it answers (§12.7) ────────────────────────────────

/// `data.employees` is exactly the `/api/employees` `list` answer: every account of every
/// branch (cross-branch, `employees.js:5`), each `{id, username, name, role, branchId, active,
/// mustChange}`, in the list's order — whichever branch is named. `version` is the named
/// branch's.
#[actix_web::test]
async fn admin_employees_answers_the_whole_list_whichever_branch_is_named() {
    let room = Room::open().await;
    let list = room.list().await;
    let rows = list.as_array().expect("a list");
    assert_eq!(rows.len(), Role::STAFF.len() + 1, "every employee: {list}");
    assert!(
        rows.iter().any(|e| e["branchId"] == json!("A1"))
            && rows.iter().any(|e| e["branchId"] == json!(MAIN)),
        "the list spans branches: {list}"
    );
    for query in ["", "branch=MAIN", "branch=A1", "branch=Z9"] {
        let body = room.view(STAFF_VIEW, "OWNER", query).await;
        assert_eq!(
            keys(&body),
            set(&["changed", "data", "version"]),
            "{query}: {body}"
        );
        assert_eq!(body["changed"], json!(true), "{query}");
        let named = query.strip_prefix("branch=").unwrap_or(MAIN);
        assert_eq!(
            body["version"],
            json!(room.version(named).await),
            "{query}: the named branch's version"
        );
        assert_eq!(
            body["data"]["employees"], list,
            "{query}: exactly the list answer, in its order"
        );
        for row in body["data"]["employees"].as_array().expect("rows") {
            assert_eq!(
                keys(row),
                set(&[
                    "id",
                    "username",
                    "name",
                    "role",
                    "branchId",
                    "active",
                    "mustChange"
                ]),
                "{query}: {row}"
            );
        }
    }
    room.finish().await;
}

/// `VERSIONED = false` (design §12.7, ticket §9 row 3): with `since` equal to the branch's
/// version the view still answers `changed: true` with the whole list.
#[actix_web::test]
async fn admin_employees_answers_the_list_even_with_an_equal_since() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    for _ in 0..2 {
        let body = room
            .view(STAFF_VIEW, "OWNER", &format!("branch=MAIN&since={v}"))
            .await;
        assert_eq!(body["changed"], json!(true), "{body}");
        assert_eq!(body["version"], json!(v), "{body}");
        assert_eq!(body["data"]["employees"], room.list().await, "{body}");
    }
    room.finish().await;
}

/// The reason for `VERSIONED = false`: a write by another tab (create, reset, disable) does not
/// move the version of the branch on screen (`Z9`, which has no employee), yet the next read
/// with the old `since` answers the list as `/api/employees` answers it after the write.
#[actix_web::test]
async fn admin_employees_follows_every_account_write_though_the_viewed_version_did_not_move() {
    let room = Room::open().await;
    let v = room.version("Z9").await;
    let query = format!("branch=Z9&since={v}");
    let before = room.view(STAFF_VIEW, "OWNER", &query).await;
    assert!(
        before["data"]["employees"]
            .as_array()
            .is_some_and(|l| l.iter().all(|e| e["username"] != json!("newfront"))),
        "{before}"
    );

    let id = room.create("newfront", "FRONT", "A1").await;
    let after_create = room.view(STAFF_VIEW, "OWNER", &query).await;
    assert_eq!(after_create["changed"], json!(true), "{after_create}");
    assert_eq!(
        after_create["data"]["employees"],
        room.list().await,
        "create"
    );
    let row = |body: &Value| {
        body["data"]["employees"]
            .as_array()
            .and_then(|l| l.iter().find(|e| e["id"] == json!(id)).cloned())
            .unwrap_or_else(|| panic!("the new account is listed: {body}"))
    };
    assert_eq!(row(&after_create)["active"], json!(true), "{after_create}");

    room.employees("reset", json!({ "id": id, "password": "another-pass-02" }))
        .await;
    let after_reset = room.view(STAFF_VIEW, "OWNER", &query).await;
    assert_eq!(after_reset["data"]["employees"], room.list().await, "reset");

    room.employees("disable", json!({ "id": id })).await;
    let after_disable = room.view(STAFF_VIEW, "OWNER", &query).await;
    assert_eq!(
        after_disable["data"]["employees"],
        room.list().await,
        "disable"
    );
    assert_eq!(
        row(&after_disable)["active"],
        json!(false),
        "{after_disable}"
    );

    assert_eq!(
        room.version("Z9").await,
        v,
        "no account write moved Z9's version (why the view is not versioned)"
    );
    room.finish().await;
}

// ── admin_operations: who may read (W8, ticket §8) ──────────────────────────

#[actix_web::test]
async fn admin_operations_asks_nobody_signed_in_to_sign_in() {
    let room = Room::open().await;
    let answer = room.get("", &path(OPS_VIEW, "branch=MAIN")).await;
    assert_eq!(answer.status, 401, "{:.300}", answer.text);
    assert_eq!(
        answer.json(),
        json!({ "error": { "code": "signIn", "message": SIGN_IN } })
    );
    room.finish().await;
}

/// The view answers exactly whom `Policy::allow(_, Rpc, "adminSnapshot")` answers, for every
/// caller kind, each with the operations of the branch the snapshot would answer them: staff
/// their own, whatever they name (`Policy::branch`).
#[actix_web::test]
async fn admin_operations_answers_exactly_whom_the_admin_snapshot_row_allows() {
    let room = Room::open().await;
    room.seed_entity("A1", "Alpha Co.", "Alpha Desk").await;
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        // OR-4b (#767): the route table's rule, naming A1 (D-4: an employee of MAIN is refused)
        let policy = blueprint_rule(actor.as_ref(), EntryPath::Rpc, "adminSnapshot", Some("A1"));
        let answer = room.get(&who, &path(OPS_VIEW, "branch=A1")).await;
        let what = format!("{OPS_VIEW} as {who:?}");
        match &policy {
            Ok(()) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["changed"], json!(true), "{what}: {body}");
                let branch = Policy::branch(actor.as_ref(), Some(&BranchId::new("A1")));
                let owner_reads = room.operations("OWNER", branch.as_str()).await;
                assert_operations_match(&body["data"], &owner_reads, &what);
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
    // D-4: the owner reads it, alone or beside an employee; every employee of MAIN is refused A1
    assert_eq!(allowed, 1 + Role::STAFF.len(), "the owner, alone or not");
    assert!(refused >= 1, "nobody signed in is refused");
    room.finish().await;
}

// ── admin_operations: what it answers ───────────────────────────────────────

/// Before anything is set, every branch answers `{entity: null}` — the snapshot's. (CA-10 #626 Q8
/// A: no `enabled`.)
#[actix_web::test]
async fn admin_operations_answers_the_snapshots_entity_and_nothing_else() {
    let room = Room::open().await;
    for branch in [MAIN, "A1", "Z9"] {
        let body = room
            .view(OPS_VIEW, "OWNER", &format!("branch={branch}"))
            .await;
        assert_eq!(
            keys(&body),
            set(&["changed", "data", "version"]),
            "{branch}: {body}"
        );
        assert_eq!(body["changed"], json!(true), "{branch}");
        assert_eq!(
            body["version"],
            json!(room.version(branch).await),
            "{branch}"
        );
        assert_eq!(
            body["data"],
            json!({ "entity": null }),
            "{branch}: nothing set yet"
        );
        let operations = room.operations("OWNER", branch).await;
        assert_operations_match(&body["data"], &operations, branch);
    }
    room.finish().await;
}

/// Each branch answers its own: an entity set on `A1` shows on `A1` only, and staff of `A1`
/// read `A1`'s whatever branch they name.
#[actix_web::test]
async fn admin_operations_answers_the_named_branch_and_staff_their_own() {
    let room = Room::open().await;
    room.seed_entity("A1", "Alpha Co.", "Alpha Desk").await;
    let a1 = room.view(OPS_VIEW, "OWNER", "branch=A1").await;
    assert_eq!(a1["data"]["entity"]["company"], json!("Alpha Co."), "{a1}");
    assert_eq!(a1["data"]["entity"]["site"], json!("Alpha Desk"), "{a1}");
    assert_operations_match(&a1["data"], &room.operations("OWNER", "A1").await, "A1");

    let main = room.view(OPS_VIEW, "OWNER", "branch=MAIN").await;
    assert_eq!(main["data"]["entity"], Value::Null, "MAIN has none: {main}");

    // OR-4b (D-4): staff of A1 naming another branch are refused; naming none or A1 they read A1's
    for query in ["branch=MAIN", "branch=Z9"] {
        assert_out_of_scope(&room.get("FRONT_A1", &path(OPS_VIEW, query)).await, query);
    }
    for query in ["", "branch=A1"] {
        let staff = room.view(OPS_VIEW, "FRONT_A1", query).await;
        assert_eq!(staff["data"], a1["data"], "FRONT of A1 with {query:?}");
        assert_eq!(
            staff["version"],
            json!(room.version("A1").await),
            "FRONT of A1 with {query:?}: A1's version"
        );
    }
    room.finish().await;
}

/// Versioned (design §4.2, ticket §9 row 3): an equal `since` answers `changed: false` with no
/// data. Re-expected for CA-10 #626 by the ticket author: the view's two writers (`/api/employees`
/// `entity` and `enable`) are retired (Q7, design §6), so the half that followed them retires too.
#[actix_web::test]
async fn admin_operations_answers_an_equal_since_with_no_data() {
    let room = Room::open().await;
    let v0 = room.version(MAIN).await;
    let same = room
        .view(OPS_VIEW, "OWNER", &format!("branch=MAIN&since={v0}"))
        .await;
    assert_eq!(
        same,
        json!({ "version": v0, "changed": false }),
        "nothing changed: O(1), no data"
    );
    room.finish().await;
}

// ── the generated types (§5.1) ───────────────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "xc-v3-employees-{what}-{}-{}",
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

fn exported(view_name: &str) -> String {
    let view = VIEWS
        .iter()
        .find(|v| v.name == view_name)
        .unwrap_or_else(|| {
            panic!(
                "{view_name} is in the generated view table: {:?}",
                VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
            )
        });
    let scratch = Scratch::new(view_name);
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {view_name}: {e}"));
    std::fs::read_to_string(scratch.0.join(format!("{view_name}.ts")))
        .unwrap_or_else(|e| panic!("{view_name} writes {view_name}.ts: {e}"))
}

fn committed(view_name: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../web/src/shared/api/generated/{view_name}.ts"));
    std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/{view_name}.ts: {e}"))
}

#[test]
fn the_employees_type_carries_the_list_rows() {
    let ts = exported(STAFF_VIEW);
    assert!(ts.contains("employees: Array<"), "employees:\n{ts}");
    for field in [
        "id: string",
        "username: string",
        "name: string",
        "role: string",
        "branchId: string",
        "active: boolean",
        "mustChange: boolean",
    ] {
        assert!(ts.contains(field), "{field}\n{ts}");
    }
    assert!(!ts.contains("password"), "no password field\n{ts}");
    assert!(!ts.contains("bigint"), "no bigint\n{ts}");
}

/// Re-expected for CA-10 #626 by the ticket author (ruling Q8 A): `enabled` leaves the type.
#[test]
fn the_operations_type_carries_a_nullable_entity_and_no_enabled() {
    let ts = exported(OPS_VIEW);
    assert!(!ts.contains("enabled"), "no enabled\n{ts}");
    assert!(
        ts.lines()
            .any(|l| l.contains("entity: ") && l.contains(" | null")),
        "entity: <type> | null\n{ts}"
    );
    assert!(ts.contains("company: string"), "company: string\n{ts}");
    assert!(ts.contains("site: string"), "site: string\n{ts}");
    for unbounded in ["events", "requests", "attendance"] {
        assert!(!ts.contains(unbounded), "no {unbounded}\n{ts}");
    }
    assert!(!ts.contains("bigint"), "no bigint\n{ts}");
}

#[test]
fn the_generated_files_are_committed() {
    for view_name in [STAFF_VIEW, OPS_VIEW] {
        assert_eq!(
            committed(view_name),
            exported(view_name),
            "{view_name}.ts is what the DTOs generate"
        );
    }
}
