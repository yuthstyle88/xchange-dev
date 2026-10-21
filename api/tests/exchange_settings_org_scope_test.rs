//! Ticket #2397 — `PUT /api/exchange-settings` (`exchangeSettingsSave`) changes the
//! organisation-wide quote lifetime (`quoteTtlSeconds`) and therefore requires an
//! organisation-wide `SystemSettings` grant (`BranchScope::All`). A branch-scoped `ADMIN@XC-B01`
//! is refused with 403 `outOfScope`, leaving the stored TTL and audit log untouched.
//!
//! Behaviour table (§9):
//! - E1 (bug): `ADMIN@XC-B01` `{"quoteTtlSeconds":3600}` → 403 `outOfScope`; the organisation's
//!   TTL is unchanged and no audit event is written.
//! - E2: `ADMIN@*` `{"quoteTtlSeconds":600}` → 200 `{"quoteTtlSeconds":600}`.
//! - E3: `TELLER@XC-B01` `{"quoteTtlSeconds":600}` → refused by policy seam as today.
//! - E4: `ADMIN@*` with a TTL outside 30..=3600 → 400 `validationFailed` as today.

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
const SETTINGS: &str = "/api/exchange-settings";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
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
        let schema = unique("exsettingsorgscope");
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

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// The organisation's stored TTL; 300 when it has no row (§4.3 step 5).
    async fn ttl(&self) -> i64 {
        let org = xchange::presentation::pages::SEEDED_XC_ORG_ID;
        self.text(&format!(
            "SELECT max(quote_ttl_seconds)::text FROM exchange_settings WHERE organization_id = '{org}'"
        ))
        .await
        .map(|t| t.parse().unwrap())
        .unwrap_or(300)
    }

    async fn audit_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM audit_events").await
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

// ------------------------------------------------------------ callers

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row.
struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

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

enum Key {
    Fresh,
}

async fn post(db: &TestDb, who: &str, method: Method, uri: &str, body: Value, key: Key) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
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
        .insert_header((
            "idempotency-key",
            match key {
                Key::Fresh => unique("key"),
            },
        ))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

// ------------------------------------------------------------ E1–E4

#[actix_web::test]
async fn e1_branch_scoped_admin_is_refused_with_403_out_of_scope_and_ttl_stays_unchanged() {
    let db = TestDb::start().await;
    let before_ttl = db.ttl().await;
    let before_audits = db.audit_rows().await;

    let a = post(
        &db,
        "ADMIN@XC-B01",
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 3600 }),
        Key::Fresh,
    )
    .await;

    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "E1: branch-scoped ADMIN@XC-B01 must be refused with 403 outOfScope: {}",
        a.text
    );
    assert_eq!(
        db.ttl().await,
        before_ttl,
        "E1: the organisation's TTL must remain unchanged"
    );
    assert_eq!(
        db.audit_rows().await,
        before_audits,
        "E1: no audit row may be written on refusal"
    );
    db.finish().await;
}

#[actix_web::test]
async fn e2_org_wide_admin_updates_quote_ttl_seconds() {
    let db = TestDb::start().await;
    let before_audits = db.audit_rows().await;

    let a = post(
        &db,
        "ADMIN@*",
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 600 }),
        Key::Fresh,
    )
    .await;

    assert_eq!(
        a.status,
        StatusCode::OK,
        "E2: ADMIN@* must succeed with 200: {}",
        a.text
    );
    assert_eq!(
        a.body,
        json!({ "quoteTtlSeconds": 600 }),
        "E2: response body must carry updated quoteTtlSeconds"
    );
    assert_eq!(db.ttl().await, 600, "E2: stored TTL must be 600");
    assert_eq!(
        db.audit_rows().await,
        before_audits + 1,
        "E2: one audit event written"
    );
    db.finish().await;
}

#[actix_web::test]
async fn e3_teller_is_refused_by_the_policy_seam() {
    let db = TestDb::start().await;
    let before_ttl = db.ttl().await;
    let before_audits = db.audit_rows().await;

    let a = post(
        &db,
        "TELLER@XC-B01",
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 600 }),
        Key::Fresh,
    )
    .await;

    assert!(
        a.status.is_client_error()
            && matches!(code(&a).as_str(), "roleNotAllowed" | "roleNotAllowedHere"),
        "E3: TELLER@XC-B01 must be refused by the policy seam, got {} {}",
        a.status,
        a.text
    );
    assert_eq!(db.ttl().await, before_ttl, "E3: TTL unchanged");
    assert_eq!(db.audit_rows().await, before_audits, "E3: no audit row");
    db.finish().await;
}

#[actix_web::test]
async fn e4_org_wide_admin_with_invalid_ttl_gets_service_validation_refusal() {
    let db = TestDb::start().await;
    let before_ttl = db.ttl().await;
    let before_audits = db.audit_rows().await;

    for invalid_ttl in [29, 3601] {
        let a = post(
            &db,
            "ADMIN@*",
            Method::PUT,
            SETTINGS,
            json!({ "quoteTtlSeconds": invalid_ttl }),
            Key::Fresh,
        )
        .await;

        assert_eq!(
            (a.status, code(&a).as_str()),
            (StatusCode::BAD_REQUEST, "validationFailed"),
            "E4 ({invalid_ttl}): ADMIN@* with invalid TTL must receive 400 validationFailed: {}",
            a.text
        );
    }

    assert_eq!(db.ttl().await, before_ttl, "E4: TTL unchanged");
    assert_eq!(db.audit_rows().await, before_audits, "E4: no audit row");
    db.finish().await;
}
