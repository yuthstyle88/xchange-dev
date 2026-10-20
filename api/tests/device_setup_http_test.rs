//! Row AG-4 (`docs/design/xchange-v3.md` §5.3, §5.4c *One agent, one drawer, one per
//! branch*, *Identity*, *The owner's page: pair, list, revoke*): `POST /api/device-setup` and
//! `GET /_xc/device-agents`. Written before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Through the server's whole route table with real sessions (`rpc_common`) and the test's
//! clock. What the routes store is read straight from row AG-1's table. Nothing here depends
//! on rows AG-2 or AG-3. This file compiles on `main` and fails there on results.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const SETUP: &str = "/api/device-setup";
const PAGE: &str = "/_xc/device-agents";
const MAIN: &str = "MAIN";
const B9: &str = "B9";

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้น";
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
/// §5.4c *One agent, one drawer, one per branch* ([required] sentence).
const BRANCH_HAS_AGENT: &str = "สาขานี้มีเครื่องคุมลิ้นชักแล้ว";

const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn t0() -> DateTime<Utc> {
    time("2026-10-08T03:00:00.000Z")
}

fn at(seconds: i64) -> DateTime<Utc> {
    t0() + Duration::seconds(seconds)
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, "Nine").await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        desk.clock.set(t0());
        Room { desk, app, cookies }
    }

    async fn ask(&self, who: &str, t: DateTime<Utc>, action: &str, payload: Value) -> Answer {
        self.desk.clock.set(t);
        let call = Call::action(ORIGIN, SETUP, action, payload);
        let call = match who {
            "nobody" => call,
            key => call.cookie(&self.cookies[key]),
        };
        self.app.send(call).await
    }

    async fn ok(&self, t: DateTime<Utc>, action: &str, payload: Value) -> Value {
        let answer = self.ask("OWNER", t, action, payload.clone()).await;
        assert_eq!(answer.status, 200, "{action} {payload}: {}", answer.text);
        assert_eq!(answer.header("cache-control"), Some("no-store"));
        answer.json()
    }

    async fn add(&self, t: DateTime<Utc>, branch: &str, name: &str) -> Value {
        self.ok(t, "add", json!({ "branchId": branch, "name": name }))
            .await
    }

    async fn agent_text(&self, id: &str, column: &str) -> Option<String> {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {column}::text FROM device_agents WHERE id = $1::uuid"
        )))
        .bind(id)
        .fetch_one(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("read {column} of {id}: {e}"))
    }

    async fn sql(&self, statement: &str) {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn staff_keys() -> Vec<String> {
    one_of_each_role()
        .into_iter()
        .map(|p| p.key)
        .filter(|k| k != "OWNER")
        .collect()
}

/// The canonical form a code is hashed in (row AG-1's `AgentToken::code_hash`).
fn code_hash(code: &str) -> String {
    sha256_hex(&code.replace('-', "").to_uppercase())
}

fn assert_code_shape(code: &str) {
    assert_eq!(code.len(), 11, "{code:?}");
    assert_eq!(&code[5..6], "-", "{code}");
    assert!(
        code.chars()
            .filter(|c| *c != '-')
            .all(|c| CROCKFORD.contains(c)),
        "{code}"
    );
}

// ── who may ──────────────────────────────────────────────────────────────────

/// `EntryPath::DeviceSetup` is the owner's only: nobody signed in → 401, every staff role →
/// 403, each with the policy's sentence; the same-origin rule applies as on `/api/devices`.
#[actix_web::test]
async fn only_the_owner_may_set_up_agents() {
    let room = Room::open().await;
    let answer = room.ask("nobody", t0(), "list", json!({})).await;
    answer.assert_refused(401, SIGN_IN, "nobody");
    // OR-4b (#767, design §4.2 C5): SystemSettings is ADMIN's too, and B1 makes SYSTEM an ADMIN;
    // SYSTEM is asked below
    for key in staff_keys().into_iter().filter(|k| k != "SYSTEM") {
        let answer = room.ask(&key, t0(), "list", json!({})).await;
        answer.assert_refused(403, OWNER_ONLY, &key);
        let answer = room
            .ask(&key, t0(), "add", json!({ "branchId": MAIN, "name": "x" }))
            .await;
        answer.assert_refused(403, OWNER_ONLY, &key);
    }
    let foreign = room
        .app
        .send(
            Call::action(ORIGIN, SETUP, "list", json!({}))
                .header("origin", "http://elsewhere.test")
                .cookie(&room.cookies["OWNER"]),
        )
        .await;
    assert_eq!(foreign.status, 400, "another origin: {}", foreign.text);
    let answer = room.ask("OWNER", t0(), "nope", json!({})).await;
    answer.assert_refused(400, BAD_REQUEST, "an unknown action, as /api/devices");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM device_agents")
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("count");
    assert_eq!(count, 0, "nothing was added");
    // C5: SYSTEM lists and adds
    let answer = room.ask("SYSTEM", t0(), "list", json!({})).await;
    assert_eq!(answer.status, 200, "SYSTEM list (C5): {:.300}", answer.text);
    let answer = room
        .ask(
            "SYSTEM",
            t0(),
            "add",
            json!({ "branchId": MAIN, "name": "x" }),
        )
        .await;
    assert_eq!(answer.status, 200, "SYSTEM add (C5): {:.300}", answer.text);
    room.finish().await;
}

// ── add, newCode ─────────────────────────────────────────────────────────────

/// `add` answers the pairing code once; only its hash is stored, valid 10 minutes.
#[actix_web::test]
async fn add_shows_the_code_once_and_stores_only_its_hash() {
    let room = Room::open().await;
    let added = room.add(at(0), MAIN, "Mac เคาน์เตอร์ 1").await;
    let id = added["agentId"].as_str().expect("agentId").to_owned();
    let code = added["code"].as_str().expect("code").to_owned();
    assert_code_shape(&code);
    assert!(added["codeExpiresAt"].is_string(), "{added}");

    assert_eq!(
        room.agent_text(&id, "branch_id").await.as_deref(),
        Some(MAIN)
    );
    assert_eq!(
        room.agent_text(&id, "name").await.as_deref(),
        Some("Mac เคาน์เตอร์ 1")
    );
    let owner_id: String = sqlx::query_scalar(
        "SELECT u.id::text FROM users u \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("read owner users.id");
    // OR-7.m G2: rule 2 (#3700)
    assert_eq!(
        room.agent_text(&id, "created_by").await.as_deref(),
        Some(owner_id.as_str())
    );
    assert_eq!(
        room.agent_text(&id, "pair_code_hash").await,
        Some(code_hash(&code))
    );
    let expires: DateTime<Utc> =
        sqlx::query_scalar("SELECT pair_code_expires_at FROM device_agents WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&room.desk.db.pool)
            .await
            .expect("read the expiry");
    assert_eq!(expires, at(600), "10 minutes");
    let stored_anywhere: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM device_agents a WHERE to_jsonb(a)::text LIKE '%' || $1 || '%' \
         OR to_jsonb(a)::text LIKE '%' || $2 || '%'",
    )
    .bind(&code)
    .bind(code.replace('-', ""))
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("search");
    assert_eq!(stored_anywhere, 0, "the code itself is stored nowhere");

    let listed = room.ok(at(1), "list", json!({})).await;
    assert!(
        !listed.to_string().contains(&code.replace('-', "")) && !listed.to_string().contains(&code),
        "list never shows the code again: {listed}"
    );
    room.finish().await;
}

/// Follow-up 2 of the review of PR 324: the one-active-agent rule counts an agent that is
/// still PENDING (added, never paired), so it is the **second add** that is refused — not the
/// second pairing. Another branch is free; after a revoke the branch is free again.
#[actix_web::test]
async fn a_second_agent_in_a_branch_is_refused_at_add() {
    let room = Room::open().await;
    let first = room.add(at(0), MAIN, "Mac 1").await;
    let answer = room
        .ask(
            "OWNER",
            at(1),
            "add",
            json!({ "branchId": MAIN, "name": "Mac 2" }),
        )
        .await;
    answer.assert_refused(400, BRANCH_HAS_AGENT, "the first is PENDING and counts");
    room.add(at(2), B9, "Mac 9").await;
    room.ok(at(3), "revoke", json!({ "agentId": first["agentId"] }))
        .await;
    room.add(at(4), MAIN, "Mac 2").await;

    let answer = room
        .ask(
            "OWNER",
            at(5),
            "add",
            json!({ "branchId": "NOPE", "name": "x" }),
        )
        .await;
    answer.assert_refused(400, NO_SUCH_BRANCH, "a branch that does not exist");
    room.finish().await;
}

/// `newCode` gives the same agent a new code (re-pair); a revoked or unknown agent has none.
#[actix_web::test]
async fn new_code_replaces_the_code_of_the_same_agent() {
    let room = Room::open().await;
    let added = room.add(at(0), MAIN, "Mac 1").await;
    let id = added["agentId"].as_str().expect("agentId").to_owned();
    let renewed = room.ok(at(100), "newCode", json!({ "agentId": id })).await;
    assert_eq!(renewed["agentId"], id.as_str());
    let code = renewed["code"].as_str().expect("code").to_owned();
    assert_code_shape(&code);
    assert_ne!(code, added["code"].as_str().unwrap_or(""), "a new code");
    assert_eq!(
        room.agent_text(&id, "pair_code_hash").await,
        Some(code_hash(&code))
    );

    room.ok(at(200), "revoke", json!({ "agentId": id })).await;
    for agent in [
        id.as_str(),
        "a0000000-0000-4000-8000-0000000000ff",
        "not-a-uuid",
    ] {
        let answer = room
            .ask("OWNER", at(300), "newCode", json!({ "agentId": agent }))
            .await;
        assert_eq!(answer.status, 400, "{agent}: {}", answer.text);
    }
    room.finish().await;
}

// ── list ─────────────────────────────────────────────────────────────────────

/// Every agent with its state: PENDING until paired, ONLINE when seen within 60 s, OFFLINE,
/// REVOKED; with what it reported and never a hash.
#[actix_web::test]
async fn list_shows_each_agent_with_its_state() {
    let room = Room::open().await;
    let pending = room.add(at(0), MAIN, "Pending").await["agentId"].clone();
    let online = room.add(at(0), B9, "Online").await["agentId"].clone();
    room.sql(&format!(
        "UPDATE device_agents SET token_hash = 'h-online', paired_at = '{p}', \
         last_seen_at = '{s}', agent_version = '1.0.1', agent_build = 'abc1234', \
         agent_os = 'macos-14.6-arm64', agent_host = 'counter-9.local', \
         drawer_transport = 'CUPS', drawer_target = 'POS80_' WHERE id = '{online}'",
        p = at(10).to_rfc3339(),
        s = at(100).to_rfc3339(),
        online = online.as_str().expect("id")
    ))
    .await;
    room.ok(at(1), "revoke", json!({ "agentId": pending }))
        .await;
    let offline = room.add(at(2), MAIN, "Offline").await["agentId"].clone();
    room.sql(&format!(
        "UPDATE device_agents SET token_hash = 'h-offline', paired_at = '{p}', last_seen_at = '{s}' \
         WHERE id = '{id}'",
        p = at(10).to_rfc3339(),
        s = at(39).to_rfc3339(),
        id = offline.as_str().expect("id")
    ))
    .await;

    let listed = room.ok(at(100 + 60), "list", json!({})).await;
    let agents = listed["agents"].as_array().expect("agents: [...]");
    assert_eq!(agents.len(), 3, "{listed}");
    let by_id = |id: &Value| {
        agents
            .iter()
            .find(|a| a["agentId"] == *id)
            .unwrap_or_else(|| panic!("{id} listed: {listed}"))
            .clone()
    };
    assert_eq!(by_id(&pending)["state"], "REVOKED");
    assert_eq!(by_id(&offline)["state"], "OFFLINE", "seen 121 s ago");
    let on = by_id(&online);
    assert_eq!(on["state"], "ONLINE", "seen exactly 60 s ago");
    for (key, expected) in [
        ("branchId", "B9"),
        ("name", "Online"),
        ("version", "1.0.1"),
        ("build", "abc1234"),
        ("os", "macos-14.6-arm64"),
        ("host", "counter-9.local"),
        ("drawerTransport", "CUPS"),
        ("drawerTarget", "POS80_"),
    ] {
        assert_eq!(on[key], expected, "{key}: {on}");
    }
    assert!(on["lastSeenAt"].is_string(), "{on}");
    let text = listed.to_string();
    assert!(
        !text.contains("h-online") && !text.contains("hash"),
        "no hash: {text}"
    );

    room.finish().await;
}

// ── revoke ───────────────────────────────────────────────────────────────────

/// `revoke`: the agent is revoked by the owner; the branch's `QUEUED` drawer jobs become
/// `FAILED` `AGENT_REVOKED` with a `FAILED` line each; receipts, `LEASED` jobs and another
/// branch's jobs stay; a second revoke is refused.
#[actix_web::test]
async fn revoke_fails_the_branch_s_queued_drawer_jobs() {
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
    let id = room.add(at(0), MAIN, "Mac 1").await["agentId"]
        .as_str()
        .expect("id")
        .to_owned();
    for (job, branch, kind, status) in [
        ("q-1", MAIN, "DRAWER_REQUEST", "QUEUED"),
        ("r-1", MAIN, "PRINT_RECEIPT", "QUEUED"),
        ("l-1", MAIN, "DRAWER_REQUEST", "LEASED"),
        ("q-9", B9, "DRAWER_REQUEST", "QUEUED"),
    ] {
        room.sql(&format!(
            "INSERT INTO device_jobs (id, request_id, request_hash, branch_id, ref, kind, status, \
             actor, created_at, updated_at) VALUES ('{job}', '{job}', 'h', '{branch}', '', '{kind}', \
             '{status}', 'front1', now(), now())"
        ))
        .await;
    }
    let lines_before: i64 = sqlx::query_scalar("SELECT count(*) FROM device_events")
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("count");

    room.ok(at(10), "revoke", json!({ "agentId": id })).await;

    assert!(room.agent_text(&id, "revoked_at").await.is_some());
    // OR-7.m G2: rule 2 (#3700)
    assert_eq!(
        room.agent_text(&id, "revoked_by").await.as_deref(),
        Some(owner_id.as_str())
    );
    let jobs: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, status, outcome_reason FROM device_jobs ORDER BY id")
            .fetch_all(&room.desk.db.pool)
            .await
            .expect("read jobs");
    assert_eq!(
        jobs,
        vec![
            ("l-1".to_owned(), "LEASED".to_owned(), None),
            (
                "q-1".to_owned(),
                "FAILED".to_owned(),
                Some("AGENT_REVOKED".to_owned())
            ),
            ("q-9".to_owned(), "QUEUED".to_owned(), None),
            ("r-1".to_owned(), "QUEUED".to_owned(), None),
        ]
    );
    let lines: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT kind, actor, detail->>'job' FROM device_events ORDER BY seq OFFSET $1",
    )
    .bind(lines_before)
    .fetch_all(&room.desk.db.pool)
    .await
    .expect("read lines");
    // OR-7.m G2: rule 2 (#3700)
    assert!(
        lines.contains(&("FAILED".to_owned(), owner_id, "q-1".to_owned())),
        "a FAILED line for the failed job, by the owner: {lines:?}"
    );

    let again = room
        .ask("OWNER", at(11), "revoke", json!({ "agentId": id }))
        .await;
    assert_eq!(again.status, 400, "revoked once: {}", again.text);
    room.finish().await;
}

// ── the page ─────────────────────────────────────────────────────────────────

/// `GET /_xc/device-agents`: the owner gets the compiled-in page that calls
/// `/api/device-setup`; anyone else is sent to `/login`, as the kept pages are; other methods
/// are the plain 404.
#[actix_web::test]
async fn the_owner_s_page_is_served_to_the_owner_only() {
    let room = Room::open().await;
    let get = |cookie: Option<&str>| {
        let mut call = Call::post(ORIGIN, PAGE, "")
            .without("x-xchange-request")
            .without("content-type");
        call.method = actix_web::http::Method::GET;
        match cookie {
            Some(c) => call.cookie(c),
            None => call,
        }
    };
    let page = room.app.send(get(Some(&room.cookies["OWNER"]))).await;
    assert_eq!(page.status, 200, "{:.300}", page.text);
    assert!(
        page.header("content-type")
            .is_some_and(|c| c.starts_with("text/html")),
        "{:?}",
        page.header("content-type")
    );
    assert!(page.text.contains(SETUP), "the page calls {SETUP}");
    assert!(
        page.header("cache-control")
            .is_some_and(|c| c.contains("no-store")),
        "never cached"
    );

    let mut who: Vec<Option<String>> = vec![None];
    who.extend(
        staff_keys()
            .into_iter()
            .map(|k| Some(room.cookies[&k].clone())),
    );
    for cookie in who {
        let answer = room.app.send(get(cookie.as_deref())).await;
        assert_eq!(answer.status, 303, "{cookie:?}: {:.200}", answer.text);
        assert!(
            answer
                .header("location")
                .is_some_and(|l| l.ends_with("/login")),
            "{cookie:?}: {:?}",
            answer.header("location")
        );
    }

    let post = room
        .app
        .send(Call::post(ORIGIN, PAGE, "").cookie(&room.cookies["OWNER"]))
        .await;
    assert_eq!(post.status, 404, "only GET");
    room.finish().await;
}
