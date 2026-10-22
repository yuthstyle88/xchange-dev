//! #4554 (#4512 D): `userGrant` role refusals (`GrantRefusal::ProtectedRole`,
//! `GrantRefusal::ProtectedTarget`, `GrantRefusal::EnableNeedsExecutive`) answer `400 Bad Request`
//! with `error.code = "roleNotAllowed"` via `V3Error::from(PolicyError::RoleNotAllowed)`, while
//! `selfChange` and `outOfScope` remain `403 Forbidden`.

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
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_roles};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG_ADMIN: &str = "ADMIN@*";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("ug400");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches");
        Self { pool, schema }
    }

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct HeaderUsers;

fn user_id(spec: &str) -> String {
    match spec.split_once('#') {
        Some((_, id)) => id.to_owned(),
        None => format!("user-{}", spec.replace(['@', '*', '+'], "-")),
    }
}

fn user_of(spec: &str) -> Option<User> {
    let roles_part = spec.split_once('#').map_or(spec, |(r, _)| r);
    let mut roles = Vec::new();
    for one in roles_part.split('+') {
        let (role, branch) = one.split_once('@')?;
        let role = match role {
            "FINANCE" => Role::Finance,
            "ADMIN" => Role::Admin,
            "EXECUTIVE" => Role::Executive,
            "BRANCH_MANAGER" => Role::BranchManager,
            "TELLER" => Role::Teller,
            other => panic!("unknown role {other}"),
        };
        let branch = (branch != "*").then(|| BranchId::new(branch));
        roles.push(ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        ));
    }
    Some(User::new(
        &user_id(spec),
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        roles,
        None,
    ))
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
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

async fn post_grant(
    db: &TestDb,
    who: &str,
    user: &str,
    role: &str,
    branch: Option<&str>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let payload = json!({ "userId": user, "role": role, "branch": branch });
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri("/api/users/grant")
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(payload.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn a_user(db: &TestDb, status: &str, roles: &[(&str, Option<&str>)]) -> String {
    let name = unique("u");
    let user = db
        .text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, '{name}', '{status}', 'test' FROM branches WHERE id = '{B01}' \
             RETURNING id::text"
        ))
        .await
        .expect("a user of XC-B01's organisation");
    let rows: Vec<RoleRow<'_>> = roles
        .iter()
        .map(|&(role_code, branch_id)| RoleRow {
            role_code,
            branch_id,
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        })
        .collect();
    let mut conn = db.pool.acquire().await.expect("acquire connection");
    seed_roles(
        &mut conn,
        &user,
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        &rows,
    )
    .await
    .expect("seed user roles");
    user
}

#[actix_web::test]
async fn user_grant_role_refusal_answers_400_role_not_allowed() {
    let db = TestDb::start().await;
    let teller = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let exec = a_user(&db, "ACTIVE", &[("EXECUTIVE", None)]).await;

    // ProtectedRole: ADMIN without EXECUTIVE tries to grant ADMIN or EXECUTIVE -> 400 roleNotAllowed
    let a = post_grant(&db, ORG_ADMIN, &teller, "ADMIN", Some(B01)).await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "ProtectedRole: {}",
        a.text
    );
    assert_eq!(a.body["error"]["code"], "roleNotAllowed", "{}", a.text);
    assert_eq!(
        a.body["error"]["message"],
        PolicyError::RoleNotAllowed.to_string(),
        "{}",
        a.text
    );

    // ProtectedTarget: ADMIN without EXECUTIVE tries to grant a role to an EXECUTIVE -> 400 roleNotAllowed
    let b = post_grant(&db, ORG_ADMIN, &exec, "FINANCE", Some(B01)).await;
    assert_eq!(
        b.status,
        StatusCode::BAD_REQUEST,
        "ProtectedTarget: {}",
        b.text
    );
    assert_eq!(b.body["error"]["code"], "roleNotAllowed", "{}", b.text);

    db.finish().await;
}
