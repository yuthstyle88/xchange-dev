//! Ticket #1585 (SP-1) rework: test cross-organisation isolation for `org_starting_prices` view.
//!
//! Held to the lane controller claude/ad05bf8d's ruling:
//! • (a) org XC saves USD; a second organisation with no branch reads org_starting_prices -> data: []
//! • (b) that second organisation saves its own USD and then reads it -> it sees only its own entry, with no branch needed
//! • (c) XC still sees only XC's entry.

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
use uuid::Uuid;
use xc_auth::domain::access::{Actor as AccessActor, Role as AccessRole, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

const CMD_SAVE: &str = "/api/v3/commands/org_starting_price_save";
const VIEW_PRICES: &str = "/api/v3/views/org_starting_prices";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
    org: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("sp1orgisolation");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations to test schema");

        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02'), ('XC-B09', 'B09')",
        )
        .execute(&pool)
        .await
        .expect("insert branches");

        let org: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'XC-B01'")
                .fetch_one(&pool)
                .await
                .expect("the branches' organisation");

        Self { pool, schema, org }
    }

    async fn finish(self) {
        let TestDb { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderUsers {
    org: String,
}

fn user_id(spec: &str) -> String {
    let n = match spec {
        "owner" | "EXECUTIVE@*" => 1,
        "admin" | "ADMIN@*" => 2,
        other => panic!("no user id written for {other}"),
    };
    format!("00000000-0000-4000-8000-{n:012}")
}

fn user_of(spec: &str, org: &str) -> Option<AccessActor> {
    let (role, branch) = match spec {
        "owner" => ("EXECUTIVE", "*"),
        "admin" => ("ADMIN", "*"),
        other => other.split_once('@')?,
    };
    let role = match role {
        "EXECUTIVE" => AccessRole::Executive,
        "ADMIN" => AccessRole::Admin,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(AccessActor::new(
        &user_id(spec),
        org,
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
        let org = self.org.clone();
        Box::pin(async move {
            let user = user_of(&spec, &org);
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

async fn call_as_org(
    db: &TestDb,
    who: &str,
    org: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers {
        org: org.to_owned(),
    });
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn entry(currency: &str, buy: Option<&str>, sell: Option<&str>) -> Value {
    json!({
        "currency": currency,
        "buy": buy,
        "sell": sell,
    })
}

fn save_payload(prices: Vec<Value>, reason: &str) -> Value {
    json!({
        "prices": prices,
        "reason": reason,
    })
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn org_starting_prices_org_isolation() {
    let db = TestDb::start().await;
    let org_xc = db.org.clone();
    let org2_id = Uuid::new_v4();
    let org2 = org2_id.to_string();

    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) VALUES ($1, 'ORG2', 'Asia/Bangkok', now())",
    )
    .bind(org2_id)
    .execute(&db.pool)
    .await
    .expect("insert org2 into organizations");

    // Verify org2 has no branches
    let org2_branches: i64 =
        sqlx::query_scalar("SELECT count(*) FROM branches WHERE organization_id::text = $1")
            .bind(&org2)
            .fetch_one(&db.pool)
            .await
            .expect("count branches for org2");
    assert_eq!(org2_branches, 0, "org2 must have no branch");

    // (a) org XC saves USD; a second organisation with no branch reads org_starting_prices -> data: []
    let p_xc = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "XC initial starting prices",
    );
    let a_xc_save = call_as_org(&db, "owner", &org_xc, Method::POST, CMD_SAVE, Some(p_xc)).await;
    assert_eq!(
        a_xc_save.status,
        StatusCode::OK,
        "XC save must succeed: {}",
        a_xc_save.text
    );

    let v_org2 = call_as_org(&db, "owner", &org2, Method::GET, VIEW_PRICES, None).await;
    assert_eq!(
        v_org2.status,
        StatusCode::OK,
        "org2 view must succeed: {}",
        v_org2.text
    );
    let data_org2 = v_org2.body["data"]
        .as_array()
        .expect("data must be an array");
    assert!(
        data_org2.is_empty(),
        "org2 with no branch must see empty data, but saw: {}",
        v_org2.text
    );

    // (b) that second organisation saves its own USD and then reads it -> it sees only its own entry, with no branch needed
    let p_org2 = save_payload(
        vec![entry("USD", Some("34.50000000"), Some("35.50000000"))],
        "org2 initial starting prices",
    );
    let a_org2_save = call_as_org(&db, "owner", &org2, Method::POST, CMD_SAVE, Some(p_org2)).await;
    assert_eq!(
        a_org2_save.status,
        StatusCode::OK,
        "org2 save must succeed: {}",
        a_org2_save.text
    );

    let v_org2_after = call_as_org(&db, "owner", &org2, Method::GET, VIEW_PRICES, None).await;
    assert_eq!(
        v_org2_after.status,
        StatusCode::OK,
        "org2 view after save must succeed: {}",
        v_org2_after.text
    );
    let data_org2_after = v_org2_after.body["data"]
        .as_array()
        .expect("data must be an array");
    assert_eq!(
        data_org2_after.len(),
        1,
        "org2 must see only its own entry: {}",
        v_org2_after.text
    );
    assert_eq!(data_org2_after[0]["currency"], "USD");
    assert_eq!(data_org2_after[0]["buy"], "34.50000000");
    assert_eq!(data_org2_after[0]["sell"], "35.50000000");
    assert_eq!(data_org2_after[0]["reason"], "org2 initial starting prices");

    // (c) XC still sees only XC's entry
    let v_xc = call_as_org(&db, "owner", &org_xc, Method::GET, VIEW_PRICES, None).await;
    assert_eq!(
        v_xc.status,
        StatusCode::OK,
        "XC view must succeed: {}",
        v_xc.text
    );
    let data_xc = v_xc.body["data"].as_array().expect("data must be an array");
    assert_eq!(
        data_xc.len(),
        1,
        "XC must see only its own entry: {}",
        v_xc.text
    );
    assert_eq!(data_xc[0]["currency"], "USD");
    assert_eq!(data_xc[0]["buy"], "32.00000000");
    assert_eq!(data_xc[0]["sell"], "33.00000000");
    assert_eq!(data_xc[0]["reason"], "XC initial starting prices");

    db.finish().await;
}
