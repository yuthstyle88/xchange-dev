//! Ticket #460 (V2-shell, `docs/design/xchange-v3-web.md` §3.3, §12.4 in PR 474): the
//! `admin_branches` view the admin page's branch picker reads, and the admin page's HTML entry
//! served at `/ui/admin/`. Written before the implementation by the lane controller
//! (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/admin_branches` — a `BranchView`, `INHERITS = (EntryPath::Rpc,
//!   "adminSnapshot")`, `NoFilter`, data `{branches: [{id, name}]}`: the branches of the
//!   snapshot (`api/src/application/compat/snapshot.rs:81`), in its order, without
//!   `requestId`. Staff get only their own branch (`Policy::branch`, ticket §7 Branch, §9).
//! - `VERSIONED = false` (design §12.4): a branch is created under its own branch
//!   (`adminCreateBranch`, `rpc.rs:277`), so the version of the branch on screen does not move
//!   when the list changes, and the view answers `changed: true` with data whatever `since`
//!   says.
//! - The admin page's HTML entry was tested here at `/ui/admin/` until V2-switch (#472) moved it
//!   to `/`. Those five page tests were retired by #472's up-front test commit; their
//!   successors are in `v3_admin_switch_test.rs` (entry → `the_admin_entry_declares_root_with_the_branch_gate`,
//!   served → `root_serves_the_v3_admin_entry_once_the_table_holds_it`, sign-in and refusals →
//!   `root_refuses_as_v2s_branch_page_did`, `/ui/admin` → `the_review_path_redirects_to_root_keeping_the_query`).
//!
//! The view tests go through the server's whole route table with real sessions made at the
//! real sign-in doors (`rpc_common`); branches are inserted with explicit, distinct
//! `created_at`, never ordered by a frozen clock. Needs `TEST_DATABASE_URL`; nothing skips.

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
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::test_callers;

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";
const VIEW: &str = "admin_branches";

const MAIN: &str = "MAIN";
/// The name the migration gives `MAIN`.
const MAIN_NAME: &str = "XC-B";
const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A request id in v2's shape, different on every call.
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
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

fn rows(list: &[(&str, &str)]) -> Value {
    Value::Array(
        list.iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect(),
    )
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` (from the migration), then `Z9` and `A1` created after it in that order (so the
    /// snapshot's order is neither the ids' nor the names'); the owner and one employee of every
    /// staff role in `MAIN`, and a FRONT employee of `A1` (key `FRONT_A1`), all signed in.
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

    /// The `branches` of `adminSnapshot` for `branch`, as the owner reads it, without
    /// `requestId`.
    async fn snapshot_branches(&self, branch: &str) -> Value {
        let snapshot = self
            .rpc("OWNER", "adminSnapshot", json!({ "branchId": branch }))
            .await;
        let list = snapshot["branches"]
            .as_array()
            .unwrap_or_else(|| panic!("the snapshot has branches: {snapshot:.300}"));
        Value::Array(
            list.iter()
                .map(|b| json!({ "id": b["id"], "name": b["name"] }))
                .collect(),
        )
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

// ── who may read (W8) ────────────────────────────────────────────────────────

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
async fn nobody_signed_in_is_asked_to_sign_in() {
    let room = Room::open().await;
    let answer = room.get("", &path("branch=MAIN")).await;
    assert_eq!(answer.status, 401, "{:.300}", answer.text);
    assert_eq!(
        answer.json(),
        json!({ "error": { "code": "signIn", "message": SIGN_IN } })
    );
    room.finish().await;
}

/// The view answers exactly whom `Policy::authorize(_, Rpc, "adminSnapshot", ..)` answers, for every
/// caller kind: the owner every branch, an employee their own branch only, whatever is named.
#[actix_web::test]
async fn the_view_answers_exactly_whom_the_admin_snapshot_row_allows() {
    let room = Room::open().await;
    let everything = room.snapshot_branches(MAIN).await;
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
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in &callers {
        // OR-4b (#767): the route table's rule, naming Z9 (D-4: an employee of MAIN is refused)
        let policy = blueprint_rule(actor.as_ref(), EntryPath::Rpc, "adminSnapshot", Some("Z9"));
        let answer = room.get(who, &path("branch=Z9")).await;
        let what = format!("{VIEW} as {who:?}");
        match &policy {
            Ok(()) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["changed"], json!(true), "{what}: {body}");
                // an employee of MAIN: their own branch; the owner (alone or with an
                // employee in the same browser, `Rpc` answers the owner): every branch
                let expected = match actor
                    .as_ref()
                    .and_then(|a| a.roles.first()?.branch.as_ref())
                {
                    Some(own) => rows(&[(own.as_str(), MAIN_NAME)]),
                    None => everything.clone(),
                };
                assert_eq!(body["data"]["branches"], expected, "{what}");
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
    assert!(allowed >= 2, "the owner and at least one employee read it");
    assert!(refused >= 1, "nobody signed in is refused");
    room.finish().await;
}

// ── what it answers (§12.4) ──────────────────────────────────────────────────

/// The owner gets the snapshot's branches, in the snapshot's order (`MAIN` first, then by
/// creation), as `{id, name}` only — whichever branch is named.
#[actix_web::test]
async fn the_owner_reads_the_snapshots_branches_in_its_order_as_id_and_name() {
    let room = Room::open().await;
    let expected = rows(&[(MAIN, MAIN_NAME), ("Z9", "Zulu"), ("A1", "Alpha")]);
    assert_eq!(
        room.snapshot_branches(MAIN).await,
        expected,
        "the snapshot's own list"
    );
    for query in ["", "branch=MAIN", "branch=A1", "branch=Z9"] {
        let body = room.view("OWNER", query).await;
        assert_eq!(
            body.as_object()
                .map(|o| o.keys().map(String::as_str).collect::<BTreeSet<_>>()),
            Some(BTreeSet::from(["changed", "data", "version"])),
            "{query}: {body}"
        );
        assert_eq!(body["changed"], json!(true), "{query}");
        assert_eq!(
            body["data"],
            json!({ "branches": expected }),
            "{query}: exactly {{branches}}, each exactly {{id, name}}"
        );
    }
    room.finish().await;
}

/// A branch made by a request carries `requestId` in the snapshot; the view leaves it out.
#[actix_web::test]
async fn a_branch_made_by_a_request_is_listed_without_its_request_id() {
    let room = Room::open().await;
    let req = request_id();
    room.rpc(
        "OWNER",
        "adminCreateBranch",
        json!({ "branchId": MAIN, "requestId": req, "name": "XC-C" }),
    )
    .await;
    let snapshot = room
        .rpc("OWNER", "adminSnapshot", json!({ "branchId": MAIN }))
        .await;
    assert!(
        snapshot["branches"]
            .as_array()
            .is_some_and(|l| l.iter().any(|b| b["requestId"] == json!(req))),
        "the snapshot keeps requestId: {:.400}",
        snapshot["branches"]
    );
    let body = room.view("OWNER", "branch=MAIN").await;
    let list = body["data"]["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {body}"));
    let made: Vec<&Value> = list.iter().filter(|b| b["name"] == json!("XC-C")).collect();
    assert_eq!(made.len(), 1, "XC-C is listed once: {body}");
    let keys: BTreeSet<&str> = made[0]
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, BTreeSet::from(["id", "name"]), "{body}");
    assert_eq!(
        body["data"]["branches"],
        room.snapshot_branches(MAIN).await,
        "the snapshot's list, in its order"
    );
    room.finish().await;
}

/// Staff get their own branch only, named or not, whatever branch they name (§9).
#[actix_web::test]
async fn staff_get_only_their_own_branch_whatever_they_name() {
    let room = Room::open().await;
    // OR-4b (D-4): naming no branch or their own, an employee gets their own; naming another, they
    // are refused
    for query in ["", "branch=MAIN", "branch=Z9", "branch=A1"] {
        if query.is_empty() || query == "branch=A1" {
            let body = room.view("FRONT_A1", query).await;
            assert_eq!(
                body["data"],
                json!({ "branches": rows(&[("A1", "Alpha")]) }),
                "FRONT of A1 with {query:?}"
            );
        } else {
            assert_out_of_scope(&room.get("FRONT_A1", &path(query)).await, query);
        }
        if query.is_empty() || query == "branch=MAIN" {
            let body = room.view("FRONT", query).await;
            assert_eq!(
                body["data"],
                json!({ "branches": rows(&[(MAIN, MAIN_NAME)]) }),
                "FRONT of MAIN with {query:?}"
            );
        } else {
            assert_out_of_scope(&room.get("FRONT", &path(query)).await, query);
        }
    }
    room.finish().await;
}

// ── VERSIONED = false (§12.4) ────────────────────────────────────────────────

/// With `since` equal to the branch's version the view still answers `changed: true` with the
/// list: it is never `changed: false`.
#[actix_web::test]
async fn an_equal_since_still_answers_the_list() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    for _ in 0..2 {
        let body = room.view("OWNER", &format!("branch=MAIN&since={v}")).await;
        assert_eq!(body["changed"], json!(true), "{body}");
        assert_eq!(body["version"], json!(v), "{body}");
        assert_eq!(
            body["data"]["branches"],
            room.snapshot_branches(MAIN).await,
            "{body}"
        );
    }
    room.finish().await;
}

/// The reason for `VERSIONED = false`: a branch created from one branch (`MAIN`) does not
/// move the version of another branch on screen (`Z9`), yet the next read of `Z9` with its old
/// `since` lists the new branch. (Measured: the create does move `MAIN`'s own version, the
/// branch the request names; any other branch's stays.)
#[actix_web::test]
async fn a_new_branch_is_listed_though_the_viewed_branchs_version_did_not_move() {
    let room = Room::open().await;
    let v = room.version("Z9").await;
    let before = room.view("OWNER", &format!("branch=Z9&since={v}")).await;
    assert!(
        before["data"]["branches"]
            .as_array()
            .is_some_and(|l| l.iter().all(|b| b["name"] != json!("XC-N"))),
        "{before}"
    );
    room.rpc(
        "OWNER",
        "adminCreateBranch",
        json!({ "branchId": MAIN, "requestId": request_id(), "name": "XC-N" }),
    )
    .await;
    assert_eq!(
        room.version("Z9").await,
        v,
        "creating a branch from MAIN does not move Z9's version (why the view is not versioned)"
    );
    let after = room.view("OWNER", &format!("branch=Z9&since={v}")).await;
    assert_eq!(after["changed"], json!(true), "{after}");
    assert_eq!(after["version"], json!(v), "{after}");
    assert!(
        after["data"]["branches"]
            .as_array()
            .is_some_and(|l| l.iter().any(|b| b["name"] == json!("XC-N"))),
        "the new branch is listed: {after}"
    );
    room.finish().await;
}

// ── the generated types (§5.1) ───────────────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "xc-v3-admin-{what}-{}-{}",
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

fn exported() -> String {
    let view = VIEWS.iter().find(|v| v.name == VIEW).unwrap_or_else(|| {
        panic!(
            "{VIEW} is in the generated view table: {:?}",
            VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
        )
    });
    let scratch = Scratch::new("types");
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {VIEW}: {e}"));
    std::fs::read_to_string(scratch.0.join(format!("{VIEW}.ts")))
        .unwrap_or_else(|e| panic!("{VIEW} writes {VIEW}.ts: {e}"))
}

#[test]
fn the_generated_type_carries_branches_of_id_and_name() {
    let ts = exported();
    assert!(ts.contains("branches: Array<"), "branches:\n{ts}");
    assert!(ts.contains("id: string"), "id: string\n{ts}");
    assert!(ts.contains("name: string"), "name: string\n{ts}");
    assert!(!ts.contains("requestId"), "no requestId\n{ts}");
    assert!(!ts.contains("bigint"), "no bigint\n{ts}");
}

#[test]
fn the_generated_file_is_committed() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../web/src/shared/api/generated/{VIEW}.ts"));
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/{VIEW}.ts: {e}"));
    assert_eq!(committed, exported(), "{VIEW}.ts is what the DTOs generate");
}
