//! #4551 (#4512 A): role refusals in `cash_exception_raise`, `cash_exception_review`, and `cash`
//! answer 400 (`roleNotAllowedHere`), while `outOfScope` stays 403.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const ANY_ID: &str = "00000000-0000-0000-0000-00000000c0a8";

const TELLER_B1: &str = "TELLER@XC-B01#1";
const FINANCE_B2: &str = "FINANCE@XC-B02#2";
const MARKETING_B1: &str = "MARKETING@XC-B01#3";
/// Passes the seam's policy check with all roles on B1, but has no row in `users`,
/// reaching `cash.rs::caller_in`.
const UNSEEDED_HOLDER: &str = "ALL@XC-B01#99";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn user_id(spec: &str) -> String {
    let (_, n) = spec.rsplit_once('#').expect("a spec ends in #n");
    format!(
        "00000000-0000-4000-8000-{:012}",
        n.parse::<u64>().expect("n")
    )
}

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("r4551");
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
            .max_connections(8)
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

        for spec in [TELLER_B1, FINANCE_B2, MARKETING_B1] {
            sqlx::query(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ($1::uuid, $2::uuid, $3, 'ACTIVE', 'test')",
            )
            .bind(user_id(spec))
            .bind(ORG)
            .bind(spec)
            .execute(&pool)
            .await
            .expect("a user");
        }
        for branch in [B1, B2] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now()) ON CONFLICT (branch_id) DO NOTHING",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch version");
        }
        Self { pool, schema }
    }

    async fn finish(self) {
        let Self { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role_str, rest) = spec.split_once('@')?;
    let (branch, _) = rest.split_once('#').expect("a spec ends in #n");
    let roles: Vec<Role> = match role_str {
        "FINANCE" => vec![Role::Finance],
        "TELLER" => vec![Role::Teller],
        "MARKETING" => vec![Role::Marketing],
        "ALL" => vec![
            Role::Executive,
            Role::Admin,
            Role::BranchManager,
            Role::Teller,
            Role::Finance,
        ],
        other => panic!("unknown role {other}"),
    };
    let scoped = roles
        .into_iter()
        .map(|role| {
            ScopedRole::new(
                role,
                Some(BranchId::new(branch)),
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )
        })
        .collect();
    Some(User::new(&user_id(spec), ORG, scoped, None))
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
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

async fn call(room: &Room, who: &str, method: Method, uri: &str, body: Value) -> Answer {
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
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("k")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn assert_refused(a: &Answer, want_status: StatusCode, want_code: &str, what: &str) {
    let got_code = a.body["error"]["code"].as_str().unwrap_or_default();
    assert_eq!(
        (a.status, got_code),
        (want_status, want_code),
        "{what}: {}",
        a.text
    );
}

#[actix_web::test]
async fn role_refusals_in_cash_exception_and_cash_commands_answer_400() {
    let room = Room::start().await;

    let raise_body = json!({
        "branch": B1,
        "reason": "short",
        "place": "DRAWER",
        "differences": [{ "currency": "THB", "amount": "-10" }]
    });
    let r = call(
        &room,
        TELLER_B1,
        Method::POST,
        "/api/v3/commands/cash_exception_raise",
        raise_body.clone(),
    )
    .await;
    assert_refused(
        &r,
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "POST /api/v3/commands/cash_exception_raise as TELLER",
    );

    let review_body = json!({
        "branch": B1,
        "exceptionId": "EX-1",
        "note": "checked"
    });
    let r = call(
        &room,
        TELLER_B1,
        Method::POST,
        "/api/v3/commands/cash_exception_review",
        review_body.clone(),
    )
    .await;
    assert_refused(
        &r,
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "POST /api/v3/commands/cash_exception_review as TELLER",
    );

    // outOfScope stays 403
    let r = call(
        &room,
        FINANCE_B2,
        Method::POST,
        "/api/v3/commands/cash_exception_raise",
        raise_body,
    )
    .await;
    assert_refused(
        &r,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "POST /api/v3/commands/cash_exception_raise outOfScope",
    );
    let r = call(
        &room,
        FINANCE_B2,
        Method::POST,
        "/api/v3/commands/cash_exception_review",
        review_body,
    )
    .await;
    assert_refused(
        &r,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "POST /api/v3/commands/cash_exception_review outOfScope",
    );

    let cash_routes: Vec<(Method, String, Value)> = vec![
        (
            Method::POST,
            "/api/cash-counts".into(),
            json!({ "branch": B1, "kind": "OPEN", "lines": [] }),
        ),
        (
            Method::POST,
            format!("/api/cash-counts/{ANY_ID}/verify"),
            json!({ "branch": B1 }),
        ),
        (
            Method::POST,
            "/api/handovers".into(),
            json!({
                "branch": B1,
                "fromShiftId": "SH-1",
                "receiverId": user_id(TELLER_B1),
                "lines": []
            }),
        ),
        (
            Method::POST,
            format!("/api/handovers/{ANY_ID}/accept"),
            json!({ "branch": B1 }),
        ),
        (
            Method::POST,
            format!("/api/handovers/{ANY_ID}/reject"),
            json!({ "branch": B1 }),
        ),
        (
            Method::POST,
            "/api/cash-transfers".into(),
            json!({
                "branch": B1,
                "kind": "SAFE_TO_DRAWER",
                "lines": [{ "currency": "THB", "denominationId": 1, "qty": "1" }],
                "note": "x"
            }),
        ),
        (
            Method::POST,
            format!("/api/cash-transfers/{ANY_ID}/submit"),
            json!({ "branch": B1 }),
        ),
        (
            Method::POST,
            format!("/api/cash-transfers/{ANY_ID}/dispatch"),
            json!({ "branch": B1 }),
        ),
        (
            Method::POST,
            format!("/api/cash-transfers/{ANY_ID}/receive"),
            json!({ "branch": B1, "lines": [] }),
        ),
        (
            Method::POST,
            format!("/api/cash-transfers/{ANY_ID}/reconcile"),
            json!({ "branch": B1, "note": "x" }),
        ),
    ];

    for (method, uri, body) in &cash_routes {
        let r = call(&room, MARKETING_B1, method.clone(), uri, body.clone()).await;
        assert_refused(
            &r,
            StatusCode::BAD_REQUEST,
            "roleNotAllowedHere",
            &format!("{method} {uri} without role"),
        );

        let r = call(&room, UNSEEDED_HOLDER, method.clone(), uri, body.clone()).await;
        assert_refused(
            &r,
            StatusCode::BAD_REQUEST,
            "roleNotAllowedHere",
            &format!("{method} {uri} caller_in without users row"),
        );
    }

    let r = call(
        &room,
        MARKETING_B1,
        Method::PUT,
        "/api/cash-transfer-policy",
        json!({ "branch": B1, "limits": {} }),
    )
    .await;
    assert_refused(
        &r,
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "PUT /api/cash-transfer-policy without role",
    );

    room.finish().await;
}
