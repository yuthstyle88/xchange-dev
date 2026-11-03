//! #5618 — a v3 command that has passed its permission check does not refuse the caller again
//! through the v2 `Standing::of` (design `docs/design/org-wide-actor-branch.md` §12, rows H-1..H-8).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Callers are `x-test-as: <ROLE>@<branch>` (`@*` = organisation-wide, session at `MAIN`). An
//! `EXECUTIVE@*` alone holds `RatesDraft`, `SystemSettings` and `BranchesManage` (RBAC §11.2) and has
//! no v2 standing (`standing.rs`: EXECUTIVE alone ⇒ `None`). Today each route below answers it
//! 400 `roleNotAllowedHere` after the permission check passed.
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
        let schema = unique("standingregate");
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
        "RATE_MANAGER" => Role::RateManager,
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let session = Some(branch.clone().unwrap_or_else(|| BranchId::new("MAIN")));
    Some(
        User::new(
            &user_id(spec),
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            vec![ScopedRole::new(
                role,
                branch,
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )],
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

// ------------------------------------------------------------ H-1..H-8

fn exec() -> &'static str {
    "EXECUTIVE@*"
}

/// H-1: an EXECUTIVE alone creates a denomination group: 200, audited as `EXECUTIVE`.
#[actix_web::test]
async fn h1_an_executive_alone_creates_a_denomination_group() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        exec(),
        Method::POST,
        GROUPS,
        json!({ "currency": "USD", "group": "LARGE" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "H-1: {}", a.text);
    assert_eq!(
        a.body,
        json!({ "currency": "USD", "group": "LARGE" }),
        "H-1: {}",
        a.text
    );
    assert_eq!(
        db.audited("denominationGroupCreate").await,
        vec![(user_id(exec()), "EXECUTIVE".to_owned())],
        "H-1: one audit row, actor_role EXECUTIVE"
    );
    db.finish().await;
}

/// H-2: an EXECUTIVE alone maps, then removes, a group: 200 each, audited as `EXECUTIVE`.
#[actix_web::test]
async fn h2_an_executive_alone_maps_and_removes_a_denomination_group() {
    let db = TestDb::start().await;
    let made = post(
        &db,
        exec(),
        Method::POST,
        GROUPS,
        json!({ "currency": "USD", "group": "LARGE" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(made.status, StatusCode::OK, "H-2 create: {}", made.text);
    let mapped = post(
        &db,
        exec(),
        Method::POST,
        "/api/denomination-groups/map",
        json!({ "currency": "USD", "group": "LARGE", "provider": "RATE_BOARD", "instrumentKey": "USD1" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(mapped.status, StatusCode::OK, "H-2 map: {}", mapped.text);
    let removed = post(
        &db,
        exec(),
        Method::POST,
        "/api/denomination-groups/remove",
        json!({ "currency": "USD", "group": "LARGE" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(
        removed.status,
        StatusCode::OK,
        "H-2 remove: {}",
        removed.text
    );
    for action in ["denominationGroupMap", "denominationGroupRemove"] {
        assert_eq!(
            db.audited(action).await,
            vec![(user_id(exec()), "EXECUTIVE".to_owned())],
            "H-2: {action} audited as EXECUTIVE"
        );
    }
    db.finish().await;
}

/// H-3: a RATE_MANAGER holds `RatesDraft` but not the group rule: 403 `outOfScope`, as today.
#[actix_web::test]
async fn h3_a_rate_manager_is_still_out_of_scope_on_groups() {
    let db = TestDb::start().await;
    let before = db.audit_rows().await;
    let a = post(
        &db,
        "RATE_MANAGER@MAIN",
        Method::POST,
        GROUPS,
        json!({ "currency": "USD", "group": "LARGE" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "H-3: {}",
        a.text
    );
    assert_eq!(db.audit_rows().await, before, "H-3: no audit row");
    db.finish().await;
}

/// H-4: an EXECUTIVE alone saves the organisation's quote lifetime: 200, audited as `EXECUTIVE`.
#[actix_web::test]
async fn h4_an_executive_alone_saves_exchange_settings() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        exec(),
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 600 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "H-4: {}", a.text);
    assert_eq!(a.body, json!({ "quoteTtlSeconds": 600 }), "H-4: {}", a.text);
    assert_eq!(db.ttl().await, 600, "H-4: stored TTL");
    assert_eq!(
        db.audited("exchangeSettingsSave").await,
        vec![(user_id(exec()), "EXECUTIVE".to_owned())],
        "H-4: audited as EXECUTIVE"
    );
    db.finish().await;
}

/// H-5: an org-wide ADMIN is refused on exchange_settings (#6004: SystemSettings is EXECUTIVE only).
#[actix_web::test]
async fn h5_an_org_wide_admin_is_unchanged_on_exchange_settings() {
    // Restated for #6004 (user-management-rbac.md §11.11): SystemSettings is EXECUTIVE only; ADMIN@* is refused by the policy seam and writes no audit row.
    let db = TestDb::start().await;
    let a = post(
        &db,
        "ADMIN@*",
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 900 }),
        Key::Fresh,
    )
    .await;
    assert!(
        a.status.is_client_error()
            && matches!(code(&a).as_str(), "roleNotAllowed" | "roleNotAllowedHere"),
        "H-5: {} {}",
        a.status,
        a.text
    );
    assert_eq!(
        db.audited("exchangeSettingsSave").await,
        Vec::<(String, String)>::new(),
        "H-5: no audit row on refusal"
    );
    db.finish().await;
}

/// H-6: an EXECUTIVE alone sets its session branch's approval limit: 200, audited as `EXECUTIVE`.
#[actix_web::test]
async fn h6_an_executive_alone_saves_a_branch_exchange_limit() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        exec(),
        Method::PUT,
        LIMIT,
        json!({ "approvalLimitThb": "50000.00" }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "H-6: {}", a.text);
    assert_eq!(
        db.audited("branchExchangeLimitSave").await,
        vec![(user_id(exec()), "EXECUTIVE".to_owned())],
        "H-6: audited as EXECUTIVE"
    );
    db.finish().await;
}

/// H-7: an org-wide ADMIN is refused on the branch limit (#6004: BranchSettingsSave is EXECUTIVE only).
#[actix_web::test]
async fn h7_an_org_wide_admin_is_unchanged_on_a_branch_exchange_limit() {
    // Restated for #6004 (user-management-rbac.md §11.11): branchExchangeLimitSave requires BranchSettingsSave (EXECUTIVE only); ADMIN@* is refused.
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
    assert!(
        a.status.is_client_error()
            && matches!(code(&a).as_str(), "roleNotAllowed" | "roleNotAllowedHere"),
        "H-7: {} {}",
        a.status,
        a.text
    );
    assert_eq!(
        db.audited("branchExchangeLimitSave").await,
        Vec::<(String, String)>::new(),
        "H-7: no audit row on refusal"
    );
    db.finish().await;
}

/// H-8: a TELLER still has no `SystemSettings`: refused by the policy seam, as today.
#[actix_web::test]
async fn h8_a_teller_is_still_refused_by_the_policy_seam() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        "TELLER@MAIN",
        Method::PUT,
        SETTINGS,
        json!({ "quoteTtlSeconds": 600 }),
        Key::Fresh,
    )
    .await;
    assert!(
        a.status.is_client_error()
            && matches!(code(&a).as_str(), "roleNotAllowed" | "roleNotAllowedHere"),
        "H-8: {} {}",
        a.status,
        a.text
    );
    db.finish().await;
}
