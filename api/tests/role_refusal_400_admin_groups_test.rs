//! Regression tests for #4553 (#4512 C): `adminGroupAdd`, `adminGroupSave`, and `adminGroupDelete`
//! each answer `400 roleNotAllowed` (via `V3Error::from(PolicyError::RoleNotAllowed)`) when a caller
//! passes the `UsersManage` policy seam without a valid organisation-wide `EXECUTIVE` role.

#![allow(dead_code)]

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
use xc_auth::domain::access::permission_module::PermissionModule;
use xc_auth::domain::access::{
    Actor as User, Permission, Role, RoleGrantStore, RoleGrants, ScopedRole,
};
use xc_auth::infrastructure::access::PgRoleGrantStore;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SEEDED_XC_ORG_ID, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
        let schema = xc_testdb::unique_schema_name("rr400ag");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .execute(&pool)
            .await
            .expect("insert branch XC-B01");
        let store = PgRoleGrantStore::new();
        let mut tx = pool.begin().await.expect("begin seed");
        store
            .seed_missing(&mut tx)
            .await
            .expect("seed built-in groups");
        tx.commit().await.expect("commit seed");
        let mut conn = pool.acquire().await.expect("a connection");
        let snapshot = store.load(&mut conn).await.expect("load groups");
        RoleGrants::install(snapshot);
        Self { pool, schema }
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

fn scoped(one: &str) -> ScopedRole {
    let (group, branch) = one.split_once('@').expect("`ROLE@branch`");
    let (code, base) = group.split_once(':').unwrap_or((group, group));
    let base = Role::parse(base).expect("known role");
    let branch = (branch != "*").then(|| BranchId::new(branch));
    ScopedRole::in_group(code, base, branch, DateTime::<Utc>::UNIX_EPOCH, None)
}

fn user_of(spec: &str) -> Option<User> {
    if spec.is_empty() {
        return None;
    }
    let user_id = format!("user-{}", spec.replace(['@', '*', '+', ':'], "-"));
    let roles = spec.split('+').map(scoped).collect();
    Some(User::new(&user_id, SEEDED_XC_ORG_ID, roles, None))
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

async fn post(db: &TestDb, who: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", xc_testdb::unique_schema_name("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn teller_module_perms() -> Vec<String> {
    RoleGrants::seed(Role::Teller)
        .into_iter()
        .filter(|p: &Permission| PermissionModule::module(*p).is_some())
        .step_by(2)
        .map(|p| {
            serde_json::to_value(p)
                .expect("permission serializes")
                .as_str()
                .expect("string")
                .to_owned()
        })
        .collect()
}

#[actix_web::test]
async fn admin_group_add_refuses_non_executive_with_400_role_not_allowed() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let perms = teller_module_perms();
    let mut answers = Vec::new();
    for who in ["ADMIN@*", "EXECUTIVE@XC-B01"] {
        let (status, body) = post(
            &db,
            who,
            "/api/admin/groups/add",
            json!({
                "code": "NIGHT_TELLER",
                "name": "Night teller",
                "base": "TELLER",
                "permissions": perms,
            }),
        )
        .await;
        answers.push((who, status, body));
    }
    db.finish().await;

    for (who, status, body) in answers {
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{who} on /api/admin/groups/add: {body}"
        );
        assert_eq!(body["error"]["code"], json!("roleNotAllowed"));
    }
}

#[actix_web::test]
async fn admin_group_save_refuses_non_executive_with_400_role_not_allowed() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let perms = teller_module_perms();
    let mut answers = Vec::new();
    for who in ["ADMIN@*", "EXECUTIVE@XC-B01"] {
        let (status, body) = post(
            &db,
            who,
            "/api/admin/groups/save",
            json!({
                "code": "TELLER",
                "permissions": perms,
            }),
        )
        .await;
        answers.push((who, status, body));
    }
    db.finish().await;

    for (who, status, body) in answers {
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{who} on /api/admin/groups/save: {body}"
        );
        assert_eq!(body["error"]["code"], json!("roleNotAllowed"));
    }
}

#[actix_web::test]
async fn admin_group_delete_refuses_non_executive_with_400_role_not_allowed() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let mut answers = Vec::new();
    for who in ["ADMIN@*", "EXECUTIVE@XC-B01"] {
        let (status, body) = post(
            &db,
            who,
            "/api/admin/groups/delete",
            json!({
                "code": "NIGHT_TELLER",
            }),
        )
        .await;
        answers.push((who, status, body));
    }
    db.finish().await;

    for (who, status, body) in answers {
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{who} on /api/admin/groups/delete: {body}"
        );
        assert_eq!(body["error"]["code"], json!("roleNotAllowed"));
    }
}
