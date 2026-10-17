//! `POST /api/device-setup` keeps a branch-scoped ADMIN inside its own branch.
//!
//! Design `docs/design/blueprint/org-roles-branches.md` §4.2 row C5 gives DeviceSetup to ADMIN, and row C4 says ADMIN
//! acts "within its scope". `ScopeRule` lets an ADMIN role be tied to one branch. On `main` the handler
//! (`presentation/http/device_setup.rs`) authorises with no named branch and never looks at the grant, so an ADMIN of
//! B9 can list, add, re-pair and revoke the drawer agents of every branch.
//!
//! Written before the implementation by the sweeper claude/dell-claude-6, on the xchange BE controller
//! claude/1656900d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips. Each test works in a schema
//! of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const SETUP: &str = "/api/device-setup";
const MAIN: &str = "MAIN";
const B9: &str = "B9";

/// An ADMIN for the whole organisation.
const ORG_ADMIN: &str = "org-admin;ADMIN,*";
/// An ADMIN whose only role is tied to branch B9.
const B9_ADMIN: &str = "b9-admin;ADMIN,B9";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    /// A schema with branches MAIN and B9 and no drawer agent.
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("dsscope");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        for b in [MAIN, B9] {
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(b)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("branch {b}: {e}"));
        }
        Self { pool, schema }
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    /// `(pair_code_hash, revoked_at)` of one agent, as text.
    async fn agent_row(&self, id: &str) -> (Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT pair_code_hash, revoked_at::text FROM device_agents WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read agent {id}: {e}"))
    }

    async fn agents_in(&self, branch: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM device_agents WHERE branch_id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count agents of {branch}: {e}"))
    }
}

// ------------------------------------------------------------ callers

/// `<id>;<ROLE>,<branch or *>;…` → a Blueprint user of `ORG` whose roles started in 2026-01 and have not ended.
fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.split(';');
    let id = parts.next().filter(|s| !s.is_empty())?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let roles = parts
        .map(|p| {
            let mut f = p.split(',');
            let role = Role::parse(f.next().expect("a role")).expect("a known role");
            let branch = f.next().filter(|b| *b != "*").map(BranchId::new);
            ScopedRole::new(role, branch, from, None)
        })
        .collect();
    Some(User::new(id, ORG, roles, None))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                owner: None,
                staff: None,
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn ask(room: &Room, who: &str, action: &str, payload: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(SETUP)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.to_owned()))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(json!({ "action": action, "payload": payload }).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// Adds an agent to `branch` as the organisation-wide ADMIN and returns its id.
async fn added(room: &Room, branch: &str) -> String {
    let a = ask(
        room,
        ORG_ADMIN,
        "add",
        json!({ "branchId": branch, "name": format!("drawer {branch}") }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "add {branch}: {}", a.text);
    a.body["agentId"]
        .as_str()
        .unwrap_or_else(|| panic!("agentId: {}", a.text))
        .to_owned()
}

fn listed_branches(a: &Answer) -> Vec<String> {
    let mut branches: Vec<String> = a.body["agents"]
        .as_array()
        .unwrap_or_else(|| panic!("agents is a list: {}", a.text))
        .iter()
        .map(|g| g["branchId"].as_str().expect("branchId").to_owned())
        .collect();
    branches.sort();
    branches
}

// ------------------------------------------------------------ tests

/// Row 1 (kept): the organisation-wide ADMIN lists every branch's agents.
#[actix_web::test]
async fn an_organisation_wide_admin_lists_every_branch() {
    let room = Room::start().await;
    added(&room, MAIN).await;
    added(&room, B9).await;

    let a = ask(&room, ORG_ADMIN, "list", json!({})).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(listed_branches(&a), vec![B9.to_owned(), MAIN.to_owned()]);
    room.finish().await;
}

/// Row 2 (kept): an ADMIN of B9 adds, re-pairs and revokes B9's own agent.
#[actix_web::test]
async fn a_branch_admin_manages_its_own_branch() {
    let room = Room::start().await;
    let a = ask(
        &room,
        B9_ADMIN,
        "add",
        json!({ "branchId": B9, "name": "drawer nine" }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "add B9: {}", a.text);
    let id = a.body["agentId"].as_str().expect("agentId").to_owned();

    let a = ask(&room, B9_ADMIN, "newCode", json!({ "agentId": id })).await;
    assert_eq!(a.status, StatusCode::OK, "newCode B9: {}", a.text);

    let a = ask(&room, B9_ADMIN, "revoke", json!({ "agentId": id })).await;
    assert_eq!(a.status, StatusCode::OK, "revoke B9: {}", a.text);
    assert!(
        room.agent_row(&id).await.1.is_some(),
        "B9's agent is revoked"
    );
    room.finish().await;
}

/// Row 3: an ADMIN of B9 lists only B9's agents, not MAIN's.
#[actix_web::test]
async fn a_branch_admin_lists_only_its_own_branch() {
    let room = Room::start().await;
    added(&room, MAIN).await;
    added(&room, B9).await;

    let a = ask(&room, B9_ADMIN, "list", json!({})).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        listed_branches(&a),
        vec![B9.to_owned()],
        "an ADMIN of B9 sees MAIN's agent: {}",
        a.text
    );
    room.finish().await;
}

/// Row 4: an ADMIN of B9 cannot add an agent to MAIN; nothing is stored.
#[actix_web::test]
async fn a_branch_admin_cannot_add_an_agent_to_another_branch() {
    let room = Room::start().await;
    let a = ask(
        &room,
        B9_ADMIN,
        "add",
        json!({ "branchId": MAIN, "name": "rogue" }),
    )
    .await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(room.agents_in(MAIN).await, 0, "an agent was stored in MAIN");
    room.finish().await;
}

/// Row 5: an ADMIN of B9 cannot get a new pairing code for MAIN's agent; the stored code is unchanged.
#[actix_web::test]
async fn a_branch_admin_cannot_re_pair_another_branchs_agent() {
    let room = Room::start().await;
    let id = added(&room, MAIN).await;
    let before = room.agent_row(&id).await;

    let a = ask(&room, B9_ADMIN, "newCode", json!({ "agentId": id })).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert!(
        a.body.get("code").is_none(),
        "a pairing code was handed out: {}",
        a.text
    );
    assert_eq!(room.agent_row(&id).await, before, "MAIN's agent changed");
    room.finish().await;
}

/// Row 6: an ADMIN of B9 cannot revoke MAIN's agent; it stays active.
#[actix_web::test]
async fn a_branch_admin_cannot_revoke_another_branchs_agent() {
    let room = Room::start().await;
    let id = added(&room, MAIN).await;

    let a = ask(&room, B9_ADMIN, "revoke", json!({ "agentId": id })).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(
        room.agent_row(&id).await.1,
        None,
        "MAIN's agent was revoked"
    );
    room.finish().await;
}
