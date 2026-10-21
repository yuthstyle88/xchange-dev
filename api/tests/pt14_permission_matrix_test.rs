//! Up-front test for PT-14 (#3968, `docs/design/blueprint/pricing-tiers.md` §PT-14): the admin permission matrix
//! view (`/api/v3/views/admin_permission_matrix`, RBAC F5) lists `MarketRatesRefresh` under `RATES_STOCK` at
//! `CREATE`, like every other permission (no special case), and the built-in groups whose `RATES_STOCK` level is
//! `CREATE` or `FULL` (EXECUTIVE, RATE_MANAGER) hold it in their stored sets while the others do not.
//! Written before the implementation by the test writer claude/studio-claude-1 for the lane controller claude/a90ab16c.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `Permission::MarketRatesRefresh` does not exist before the fix, so this file does not compile until it does.
//!
//! Harness trimmed from `rbac_f5_permission_matrix_view_test.rs`: the `x-test-as` caller is `ROLE@branch` (`@*` =
//! organisation-wide); the schema runs every migration, then does what startup does (`seed_missing`, install the
//! loaded snapshot). `RoleGrants::install` is process-global, so the tests run one at a time (`SERIAL`).
//! `sessions` answers no old cookie, as `payment_settings_access_actor_test.rs` does; OR-7.m T-last drops it.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips.

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
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
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
const EXEC: &str = "EXECUTIVE@*";
const VIEW: &str = "/api/v3/views/admin_permission_matrix";

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// The serde name a permission is stored and sent by.
fn name(p: Permission) -> String {
    serde_json::to_value(p)
        .expect("a permission serialises")
        .as_str()
        .expect("a permission serialises to a string")
        .to_owned()
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
        let schema = unique("pt14");
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
        let db = Self { pool, schema };
        db.boot().await;
        db
    }

    /// What startup does (§11.5 item 2): seed every empty built-in group, then install the stored snapshot.
    async fn boot(&self) {
        let store = PgRoleGrantStore::new();
        let mut tx = self.pool.begin().await.expect("begin the seed");
        store
            .seed_missing(&mut tx)
            .await
            .expect("seed the built-in groups");
        tx.commit().await.expect("commit the seed");
        let mut conn = self.pool.acquire().await.expect("a connection");
        let snapshot = store.load(&mut conn).await.expect("load the groups");
        RoleGrants::install(snapshot);
    }

    async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options)
            .await
            .expect("connect to drop the schema");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = Role::parse(role).unwrap_or_else(|| panic!("unknown role {role}"));
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &format!("user-{}", spec.replace(['@', '*'], "-")),
        SEEDED_XC_ORG_ID,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
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

/// `GET VIEW` as `who`: answers the `200`'s `data`.
async fn matrix(db: &TestDb, who: &str) -> Value {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(VIEW)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "{who} reads the matrix: {text}");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");
    assert_eq!(body["changed"], json!(true), "{text}");
    body["data"].clone()
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn pt14_the_matrix_lists_market_rates_refresh_under_rates_stock_at_create() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let data = matrix(&db, EXEC).await;
    let refresh = name(Permission::MarketRatesRefresh);
    let rows: Vec<&Value> = data["permissions"]
        .as_array()
        .expect("permissions is an array")
        .iter()
        .filter(|p| p["name"] == json!(refresh))
        .collect();
    assert_eq!(
        rows,
        vec![&json!({
            "name": refresh,
            "module": "RATES_STOCK",
            "level": "CREATE",
            "editable": true,
        })],
        "one MarketRatesRefresh row, RATES_STOCK CREATE, editable: {data}"
    );
    db.finish().await;
}

#[actix_web::test]
async fn pt14_the_rates_stock_create_or_full_groups_hold_it_and_the_others_do_not() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let data = matrix(&db, EXEC).await;
    let refresh = json!(name(Permission::MarketRatesRefresh));
    let groups = data["groups"].as_array().expect("groups is an array");
    for (code, holds) in [
        ("EXECUTIVE", true),
        ("RATE_MANAGER", true),
        ("ADMIN", false),
        ("BRANCH_MANAGER", false),
        ("TELLER", false),
        ("FINANCE", false),
        ("MARKETING", false),
        ("VERIFICATION_REVIEWER", false),
    ] {
        let g = groups
            .iter()
            .find(|g| g["code"] == json!(code))
            .unwrap_or_else(|| panic!("group {code} in {data}"));
        let set = g["permissions"]
            .as_array()
            .unwrap_or_else(|| panic!("{code}.permissions is an array: {g}"));
        assert_eq!(
            set.contains(&refresh),
            holds,
            "{code} holds MarketRatesRefresh: {holds}; {g}"
        );
        let role = Role::parse(code).expect("a built-in group code");
        assert_eq!(
            set.contains(&refresh),
            RoleGrants::seed(role).contains(&Permission::MarketRatesRefresh),
            "{code}'s stored set agrees with its seed"
        );
    }
    db.finish().await;
}
