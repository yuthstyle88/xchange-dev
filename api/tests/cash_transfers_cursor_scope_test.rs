//! Ticket #2696 — Scoping cursor subquery on cash_transfers view.
//!
//! Behaviour table:
//! - C1: `branch=B1&before=<a transfer between B2 and B3>` -> `rows: []` (main: B1 rows older than it)
//! - C2: `branch=B1&before=<unknown uuid>` -> `rows: []` (kept)
//! - C3: `branch=B1&before=<own B1 transfer>` -> the next page, as today (kept)
//! - C4: status/kind filters with an own cursor -> unchanged (kept)

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";
const B03: &str = "XC-B03";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at a test database ending in `_test`")
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("ct_cursor");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        for branch in [B01, B02, B03, B09] {
            sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT DO NOTHING")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("insert a branch");
            sqlx::query(
                "INSERT INTO ops_state (branch_id, phase, safe) \
                 VALUES ($1, 'OPEN', jsonb_build_object('USD', 50000::bigint)) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(branch)
            .execute(&pool)
            .await
            .expect("the branch's safe");
        }
        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique(role.as_str()))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
             VALUES ($1::uuid, $2, $3::uuid, $4)",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("give the user a role");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct Who {
    #[allow(dead_code)]
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

fn legacy_of(spec: &str) -> Sessions {
    let mut parts = spec.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(id), Some(_), Some("*")) => Sessions {
            owner: Some(Actor::owner(id)),
            staff: None,
        },
        (Some(id), Some(_), Some(branch)) => Sessions {
            owner: None,
            staff: Actor::staff(id, OldRole::Finance, BranchId::new(branch)),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
}

struct HeaderUsers;

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move { legacy_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let s = legacy_of(&spec);
            let user = user_of(&spec);
            Callers {
                owner: s.owner,
                staff: s.staff,
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

enum Key<'a> {
    Fresh,
    #[allow(dead_code)]
    This(&'a str),
    #[allow(dead_code)]
    Missing,
}

async fn call(
    room: &Room,
    who: Option<&Who>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key<'_>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST));
    if let Some(who) = who {
        req = req.insert_header((AS, who.spec.clone()));
    }
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
        match key {
            Key::Fresh => req = req.insert_header(("idempotency-key", unique("key"))),
            Key::This(k) => req = req.insert_header(("idempotency-key", k.to_owned())),
            Key::Missing => {}
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn post(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    call(room, Some(who), Method::POST, uri, Some(body), Key::Fresh).await
}

async fn view(room: &Room, who: &Who, name: &str, query: &str) -> Answer {
    call(
        room,
        Some(who),
        Method::GET,
        &format!("/api/v3/views/{name}?branch={B01}{query}"),
        None,
        Key::Fresh,
    )
    .await
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body.clone()
}

fn data(a: &Answer, what: &str) -> Value {
    ok(a, what)["data"].clone()
}

fn id_of(a: &Answer, what: &str) -> String {
    ok(a, what)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

fn ids(page: &Value) -> Vec<String> {
    page.as_array()
        .expect("a page of records")
        .iter()
        .map(|r| r["id"].as_str().expect("every record has an id").to_owned())
        .collect()
}

// ── Behaviour table tests ───────────────────────────────────────────────────

#[actix_web::test]
async fn c1_foreign_transfer_cursor_answers_empty_page() {
    let room = Room::start().await;
    let manager_b1 = room.person(Role::BranchManager, Some(B01)).await;
    let manager_b2 = room.person(Role::BranchManager, Some(B02)).await;
    let usd50 = room.denomination("USD", "50").await;

    // Create a transfer in B1
    let b1_transfer_id = id_of(
        &post(
            &room,
            &manager_b1,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "EXTERNAL_OUT",
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "b1 transfer"
            }),
        )
        .await,
        "a B1 transfer",
    );

    // Create a transfer between B2 and B3 (foreign to B1)
    let foreign_transfer_id = id_of(
        &post(
            &room,
            &manager_b2,
            "/api/cash-transfers",
            json!({
                "branch": B02,
                "kind": "BRANCH",
                "toBranchId": B03,
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "b2 to b3 transfer"
            }),
        )
        .await,
        "a B2 to B3 transfer",
    );

    // Query B1 cash_transfers view with before = foreign transfer id
    let q = format!("&before={foreign_transfer_id}");
    let answer = view(&room, &manager_b1, "cash_transfers", &q).await;
    let rows = ids(&data(&answer, "c1 foreign cursor"));

    assert!(
        rows.is_empty(),
        "C1: foreign transfer cursor must return rows: [], got: {rows:?} (found B1 transfer: {b1_transfer_id})"
    );

    room.finish().await;
}

#[actix_web::test]
async fn c2_unknown_uuid_cursor_answers_empty_page() {
    let room = Room::start().await;
    let manager_b1 = room.person(Role::BranchManager, Some(B01)).await;
    let usd50 = room.denomination("USD", "50").await;

    // Create a transfer in B1
    let _ = id_of(
        &post(
            &room,
            &manager_b1,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "EXTERNAL_OUT",
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "b1 transfer"
            }),
        )
        .await,
        "a B1 transfer",
    );

    // Query B1 cash_transfers view with before = random unknown UUID
    let unknown_id = Uuid::new_v4().to_string();
    let q = format!("&before={unknown_id}");
    let answer = view(&room, &manager_b1, "cash_transfers", &q).await;
    let rows = ids(&data(&answer, "c2 unknown uuid cursor"));

    assert!(
        rows.is_empty(),
        "C2: unknown uuid cursor must return rows: [], got: {rows:?}"
    );

    room.finish().await;
}

#[actix_web::test]
async fn c3_own_transfer_cursor_answers_next_page() {
    let room = Room::start().await;
    let manager_b1 = room.person(Role::BranchManager, Some(B01)).await;
    let usd50 = room.denomination("USD", "50").await;

    // Create two transfers in B1
    let t1 = id_of(
        &post(
            &room,
            &manager_b1,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "EXTERNAL_OUT",
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "older transfer"
            }),
        )
        .await,
        "first B1 transfer",
    );

    let t2 = id_of(
        &post(
            &room,
            &manager_b1,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "EXTERNAL_OUT",
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "newer transfer"
            }),
        )
        .await,
        "second B1 transfer",
    );

    // Query B1 cash_transfers view with before = t2 (newer transfer)
    let q = format!("&before={t2}");
    let answer = view(&room, &manager_b1, "cash_transfers", &q).await;
    let rows = ids(&data(&answer, "c3 own transfer cursor"));

    assert_eq!(
        rows,
        vec![t1],
        "C3: own transfer cursor must answer next page containing older transfer"
    );

    room.finish().await;
}

#[actix_web::test]
async fn c4_status_and_kind_filters_with_own_cursor() {
    let room = Room::start().await;
    let manager_b1 = room.person(Role::BranchManager, Some(B01)).await;
    let usd50 = room.denomination("USD", "50").await;

    let t1 = id_of(
        &post(
            &room,
            &manager_b1,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "EXTERNAL_OUT",
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "first draft"
            }),
        )
        .await,
        "first draft",
    );

    let t2 = id_of(
        &post(
            &room,
            &manager_b1,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "EXTERNAL_OUT",
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "second draft"
            }),
        )
        .await,
        "second draft",
    );

    let t3 = id_of(
        &post(
            &room,
            &manager_b1,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "BRANCH",
                "toBranchId": B09,
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "third draft different kind"
            }),
        )
        .await,
        "third draft branch",
    );

    // Filter by kind=EXTERNAL_OUT and before=t2
    let q = format!("&kind=EXTERNAL_OUT&before={t2}");
    let answer = view(&room, &manager_b1, "cash_transfers", &q).await;
    let rows = ids(&data(&answer, "c4 filtered cursor"));

    assert_eq!(
        rows,
        vec![t1],
        "C4: filtered page with own cursor must return older matching transfer"
    );

    // Filter by kind=BRANCH and before=t3
    let q_branch = format!("&kind=BRANCH&before={t3}");
    let answer_branch = view(&room, &manager_b1, "cash_transfers", &q_branch).await;
    let rows_branch = ids(&data(&answer_branch, "c4 branch kind cursor"));
    assert!(
        rows_branch.is_empty(),
        "C4: no older BRANCH transfers exist"
    );

    room.finish().await;
}
