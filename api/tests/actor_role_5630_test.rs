//! #5630 — one rule for an audit row's `actor_role`, through the v3 commands (design
//! `docs/design/org-wide-actor-branch.md` §14.5; rulings Q-1 A and Q-2 A on #5661: the Blueprint name of the role
//! whose row granted the action). Rows R-1..R-6. Written before the implementation by the sub-controller
//! (claude/studio-claude-17). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Callers are `x-test-as: <ROLE>@<branch>[,<ROLE>@<branch>…]`, several rows in that order, so the role the grant
//! used can differ from the first row, from the `Standing` and from the joined list each writer writes today.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

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
const LIMIT: &str = "/api/branches/MAIN/exchange-limit";
const GROUPS: &str = "/api/denomination-groups";

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
        let schema = unique("actorrole5630");
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

    /// `(actor_id, actor_role)` of the audit rows of `action`, oldest first.
    async fn audited(&self, action: &str) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT actor_id, actor_role FROM audit_events WHERE action = $1 ORDER BY id",
        )
        .bind(action)
        .fetch_all(&self.pool)
        .await
        .expect("read audit_events")
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

/// A uuid per caller spec, stable within the run: some commands store the user id in a `uuid` column
/// (`branch_alert_settings_save.rs:114`).
fn user_id(spec: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    spec.hash(&mut h);
    let a = h.finish();
    uuid::Uuid::from_u64_pair(a, a.rotate_left(17)).to_string()
}

fn role_of(name: &str) -> Role {
    match name {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "RATE_MANAGER" => Role::RateManager,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        other => panic!("unknown role {other}"),
    }
}

/// `x-test-as: <ROLE>@<branch>[,<ROLE>@<branch>…]` (`@*` = organisation-wide), in that row order. The session is
/// the first row's branch, or `MAIN` for an organisation-wide first row.
fn user_of(spec: &str) -> Option<User> {
    if spec.is_empty() {
        return None;
    }
    let rows: Vec<ScopedRole> = spec
        .split(',')
        .map(|part| {
            let (role, branch) = part.split_once('@').expect("ROLE@branch");
            let branch = (branch != "*").then(|| BranchId::new(branch));
            ScopedRole::new(role_of(role), branch, DateTime::<Utc>::UNIX_EPOCH, None)
        })
        .collect();
    let session = Some(
        rows[0]
            .branch
            .clone()
            .unwrap_or_else(|| BranchId::new("MAIN")),
    );
    Some(
        User::new(
            &user_id(spec),
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            rows,
            None,
        )
        .with_branch(session),
    )
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

// ------------------------------------------------------------ R-1..R-6

const OWNER: &str = "EXECUTIVE@*,ADMIN@*";
const ALERTS: &str = "/api/v3/commands/branch_alert_settings_save";
const RAISE: &str = "/api/v3/commands/cash_exception_raise";

fn alert_body() -> Value {
    json!({ "branch": "MAIN", "staleDataMinutes": 30, "floors": [], "expectedVersion": null })
}

/// R-1 (W-12): the owner saves branch alert settings. `AlertSettingsSave` is EXECUTIVE's, not ADMIN's (RBAC
/// §11.2), so the row says `EXECUTIVE`; today it joins every role, sorted (`ADMIN,EXECUTIVE`).
#[actix_web::test]
async fn r1_branch_alert_settings_write_the_granting_role_not_a_join() {
    let db = TestDb::start().await;
    let a = post(&db, OWNER, Method::POST, ALERTS, alert_body(), Key::Fresh).await;
    assert_eq!(a.status, StatusCode::OK, "R-1: {}", a.text);
    assert_eq!(
        db.audited("BRANCH_ALERT_SETTINGS_SAVE").await,
        vec![(user_id(OWNER), "EXECUTIVE".to_owned())],
        "R-1"
    );
    db.finish().await;
}

/// R-2 (W-5): TELLER then FINANCE at `MAIN` raise a cash exception. The command's own `FinanceReconcile` check is
/// granted by the FINANCE row, so the row says `FINANCE`; today it is the first row's role (`TELLER`).
#[actix_web::test]
async fn r2_a_cash_exception_writes_the_role_that_granted_finance_reconcile() {
    let db = TestDb::start().await;
    let who = "TELLER@MAIN,FINANCE@MAIN";
    let a = post(
        &db,
        who,
        Method::POST,
        RAISE,
        json!({
            "branch": "MAIN",
            "reason": "count",
            "place": "DRAWER",
            "differences": [{ "currency": "USD", "amount": "1.00" }],
        }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "R-2: {}", a.text);
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT actor_id, actor_role FROM audit_events WHERE actor_id = $1 ORDER BY id",
    )
    .bind(user_id(who))
    .fetch_all(&db.pool)
    .await
    .expect("read audit_events");
    assert_eq!(rows, vec![(user_id(who), "FINANCE".to_owned())], "R-2");
    db.finish().await;
}

/// R-3 (W-2): the owner saves the quote lifetime. Its `SystemSettings` rows are EXECUTIVE then ADMIN; the first
/// granting row is `EXECUTIVE`. Today V.9's `OWNER`.
#[actix_web::test]
async fn r3_exchange_settings_by_the_owner_say_executive() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        OWNER,
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 600 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "R-3: {}", a.text);
    assert_eq!(
        db.audited("exchangeSettingsSave").await,
        vec![(user_id(OWNER), "EXECUTIVE".to_owned())],
        "R-3"
    );
    db.finish().await;
}

/// R-4 (W-2): an organisation-wide ADMIN sets a branch's approval limit: `ADMIN`. Today V.9's `SYSTEM`.
#[actix_web::test]
async fn r4_a_branch_limit_by_an_admin_says_admin() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        "ADMIN@*",
        Method::PUT,
        LIMIT,
        json!({ "approvalLimitThb": "50000.00" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "R-4: {}", a.text);
    assert_eq!(
        db.audited("branchExchangeLimitSave").await,
        vec![(user_id("ADMIN@*"), "ADMIN".to_owned())],
        "R-4"
    );
    db.finish().await;
}

/// R-5 (W-2): the owner creates a denomination group: `EXECUTIVE` (its `RatesDraft` row). Today V.9's `OWNER`.
#[actix_web::test]
async fn r5_a_denomination_group_by_the_owner_says_executive() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        OWNER,
        Method::POST,
        GROUPS,
        json!({ "currency": "USD", "group": "LARGE" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "R-5: {}", a.text);
    assert_eq!(
        db.audited("denominationGroupCreate").await,
        vec![(user_id(OWNER), "EXECUTIVE".to_owned())],
        "R-5"
    );
    db.finish().await;
}

/// R-6 (W-2): an EXECUTIVE alone is unchanged: `EXECUTIVE`.
#[actix_web::test]
async fn r6_an_executive_alone_is_unchanged() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        "EXECUTIVE@*",
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 900 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "R-6: {}", a.text);
    assert_eq!(
        db.audited("exchangeSettingsSave").await,
        vec![(user_id("EXECUTIVE@*"), "EXECUTIVE".to_owned())],
        "R-6"
    );
    db.finish().await;
}
