//! #4901: the cash commands refuse a bad quantity, a missing transfer end or a bad policy limit on a field that names
//! the row or currency, so the screen can put the message beside it.
//!
//! Written before the implementation by the ticket author's lane (claude/be38637e, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Rows (#4901 §3):
//! - X1. `countSubmit`, the second line's qty `1.5` → 400 `validationFailed`, field `lines[1].qty`.
//! - X2. `transferCreate`, the first line's qty `0` → 400 `validationFailed`, field `lines[0].qty`.
//! - X3. a BRANCH `transferCreate` with no `toBranchId` → 400 `validationFailed`, field `toBranch`.
//! - X4. `transferPolicySave` with a JPY limit of `100.5` (JPY has minor unit 0) → 400 `validationFailed`, field
//!   `limits.JPY`.
//! - X5. as today: a valid SAFE count, a valid BRANCH transfer and a valid policy are accepted.
//!
//! Today X1 and X2 answer field `qty`, X3 answers no field, and X4 answers field `limits`; X5 passes.
//!
//! The harness is `cash_transfer_create_overflow_test.rs`'s, trimmed. Needs `TEST_DATABASE_URL`; nothing skips.
//! Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
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
        let schema = unique("cashfields");
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
        for branch in [B01, B09] {
            sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("insert a branch");
            sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("the branch's ops_state");
        }
        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let id = seed_person(
            &mut self.pool.acquire().await.unwrap(),
            ORG,
            &unique(role.as_str()),
            &[RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: from,
                valid_to: None,
            }],
        )
        .await
        .expect("insert a user");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
        }
    }

    /// A seeded USD note of `face` units.
    async fn usd(&self, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations \
             WHERE currency_code = 'USD' AND face_value = $1::numeric ORDER BY id LIMIT 1",
        )
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the seeded USD {face} face value: {e}"))
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

struct HeaderUsers;

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
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

async fn call(room: &Room, who: &Who, method: Method, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.spec.clone()))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// `countSubmit` of a SAFE count at B01 (no shift) with the USD `lines` (`(denomination, qty)`).
async fn count_safe(room: &Room, who: &Who, lines: &[(i64, &str)]) -> Answer {
    let lines: Vec<Value> = lines
        .iter()
        .map(|(d, q)| json!({ "currency": "USD", "denominationId": d, "qty": q }))
        .collect();
    call(
        room,
        who,
        Method::POST,
        "/api/cash-counts",
        json!({ "branch": B01, "kind": "SAFE", "lines": lines }),
    )
    .await
}

/// `transferCreate` of a BRANCH transfer from B01 to `to` with the USD `lines` (`(denomination, qty)`).
async fn create(room: &Room, who: &Who, to: Option<&str>, lines: &[(i64, &str)]) -> Answer {
    let lines: Vec<Value> = lines
        .iter()
        .map(|(d, q)| json!({ "currency": "USD", "denominationId": d, "qty": q }))
        .collect();
    let mut body = json!({ "branch": B01, "kind": "BRANCH", "lines": lines, "note": "to B09" });
    if let Some(to) = to {
        body["toBranchId"] = json!(to);
    }
    call(room, who, Method::POST, "/api/cash-transfers", body).await
}

/// `transferPolicySave` of `limits`.
async fn save_policy(room: &Room, who: &Who, limits: Value) -> Answer {
    call(
        room,
        who,
        Method::PUT,
        "/api/cash-transfer-policy",
        json!({ "branch": B01, "limits": limits }),
    )
    .await
}

/// Refused as #4901 requires: 400 `validationFailed` on `field`.
fn assert_refused_on(a: &Answer, field: &str, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["code"], "validationFailed",
        "{what}: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"], field,
        "{what}: the refused field: {}",
        a.text
    );
}

fn assert_accepted(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
}

// ------------------------------------------------------------ the rows

#[actix_web::test]
async fn x1_a_count_whose_second_line_qty_is_fractional_is_refused_on_lines_1_qty() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let fifty = room.usd("50").await;
    let hundred = room.usd("100").await;

    let a = count_safe(&room, &manager, &[(fifty, "2"), (hundred, "1.5")]).await;
    assert_refused_on(&a, "lines[1].qty", "countSubmit, line 2 qty 1.5");
    room.finish().await;
}

#[actix_web::test]
async fn x2_a_transfer_whose_first_line_qty_is_zero_is_refused_on_lines_0_qty() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let fifty = room.usd("50").await;

    let a = create(&room, &manager, Some(B09), &[(fifty, "0")]).await;
    assert_refused_on(&a, "lines[0].qty", "transferCreate, line 1 qty 0");
    room.finish().await;
}

#[actix_web::test]
async fn x3_a_branch_transfer_with_no_destination_is_refused_on_to_branch() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let fifty = room.usd("50").await;

    let a = create(&room, &manager, None, &[(fifty, "10")]).await;
    assert_refused_on(&a, "toBranch", "BRANCH transferCreate with no toBranchId");
    room.finish().await;
}

#[actix_web::test]
async fn x4_a_policy_with_a_fractional_jpy_limit_is_refused_on_limits_jpy() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;

    let a = save_policy(&room, &executive, json!({ "JPY": "100.5" })).await;
    assert_refused_on(&a, "limits.JPY", "transferPolicySave, JPY limit 100.5");
    room.finish().await;
}

#[actix_web::test]
async fn x5_valid_requests_are_accepted_as_today() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    let fifty = room.usd("50").await;
    let hundred = room.usd("100").await;

    let count = count_safe(&room, &manager, &[(fifty, "2"), (hundred, "0")]).await;
    assert_accepted(&count, "countSubmit of a SAFE count, qty 2 and 0");

    let transfer = create(&room, &manager, Some(B09), &[(fifty, "10")]).await;
    assert_accepted(&transfer, "BRANCH transferCreate B01 → B09, 10 × USD 50");
    assert_eq!(
        transfer.body["status"], "DRAFT",
        "a new transfer is a DRAFT: {}",
        transfer.text
    );

    let policy = save_policy(&room, &executive, json!({ "JPY": "100", "USD": "50.5" })).await;
    assert_accepted(&policy, "transferPolicySave, JPY 100 and USD 50.5");
    room.finish().await;
}
