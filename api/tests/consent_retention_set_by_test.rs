//! CI-6 (#653): Retention policy set_by records owner's user id over HTTP.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn start() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ci6retentionsetby");
        let admin = PgPoolOptions::new()
            .max_connections(1)
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
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .as_path(),
        )
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");
        common::install_test_pii_keys();
        for b in ["MAIN", "XC-B01"] {
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(b)
            .execute(&pool)
            .await
            .expect("insert a branch");
        }
        Db { pool }
    }

    async fn user(&self) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, 'Test User', 'ACTIVE', 'test' FROM branches WHERE id = 'MAIN' \
             RETURNING id::text",
        )
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }
}

#[tokio::test]
async fn a_retention_period_set_over_http_records_the_owners_users_id_in_set_by() {
    struct CustomOwner(String);
    impl SessionReader for CustomOwner {
        fn callers<'a>(
            &'a self,
            _req: &'a HttpRequest,
        ) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
            let id = self.0.clone();
            Box::pin(async move { test_callers::callers(Some(test_callers::owner(&id)), None) })
        }
    }

    let db = Db::start().await;
    let owner_id = db.user().await;

    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(CustomOwner(owner_id.clone()));
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(Method::PUT)
        .uri("/api/retention-policies/IDENTITY_VALUE")
        .insert_header((header::HOST, HOST));
    let body = json!({"periodDays": 60});
    req = req
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let resp_body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(status, StatusCode::OK, "retention: {text}");
    assert_eq!(
        resp_body,
        json!({"dataType": "IDENTITY_VALUE", "periodDays": 60})
    );

    let set_by: Option<String> = sqlx::query_scalar(
        "SELECT set_by::text FROM retention_policies WHERE data_type = 'IDENTITY_VALUE'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("retention policy row");

    assert_eq!(set_by.as_deref(), Some(owner_id.as_str()));
}
