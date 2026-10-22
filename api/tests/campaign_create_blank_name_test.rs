//! #4789: campaignCreate refuses blank name (trim().is_empty()) with 400 validationFailed, field "name".

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{header, Method, StatusCode};
use actix_web::{test as atest, web, App, HttpRequest};
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
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
        let schema = unique("campaign_create_blank");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
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
            .expect("apply api/migrations");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, version, created_at, created_by) \
             VALUES ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed test user");

        common::install_test_pii_keys();

        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

fn marketing_of_org_a() -> Actor {
    Actor::new(
        "user-MARKETING",
        ORG_A,
        vec![ScopedRole::new(
            Role::Marketing,
            None,
            chrono::DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let as_marketing = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "MARKETING");
        let user = as_marketing.then(marketing_of_org_a);
        let staff_user = user.clone();
        Box::pin(async move { Callers { user, staff_user } })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
    }

    fn field(&self) -> &str {
        self.body["error"]["field"]
            .as_str()
            .or_else(|| self.body["field"].as_str())
            .unwrap_or("")
    }
}

async fn send_create(room: &Room, name: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let payload = json!({
        "name": name,
        "startsAt": "2026-11-01T00:00:00Z",
        "endsAt": "2026-12-01T00:00:00Z",
        "budgetThb": "10000.00",
        "stackGroup": "RATE",
        "priority": 1,
        "branches": [MAIN],
    });

    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri("/api/campaigns")
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, "MARKETING"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", unique("campaign_create_blank")))
        .set_payload(payload.to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn count_campaigns(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM campaigns")
        .fetch_one(pool)
        .await
        .expect("count campaigns")
}

#[actix_web::test]
async fn test_campaign_create_empty_name_refused() {
    let room = Room::start().await;

    let ans = send_create(&room, "").await;
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "empty name: expected 400, got {} body={}",
        ans.status,
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "empty name: expected code validationFailed");
    assert_eq!(ans.field(), "name", "empty name: expected field name");
    assert_eq!(
        count_campaigns(&room.pool).await,
        0,
        "empty name: no campaigns row may be created"
    );

    room.finish().await;
}

#[actix_web::test]
async fn test_campaign_create_whitespace_name_refused() {
    let room = Room::start().await;

    let ans = send_create(&room, "   ").await;
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "whitespace name: expected 400, got {} body={}",
        ans.status,
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "whitespace name: expected code validationFailed");
    assert_eq!(ans.field(), "name", "whitespace name: expected field name");
    assert_eq!(
        count_campaigns(&room.pool).await,
        0,
        "whitespace name: no campaigns row may be created"
    );

    room.finish().await;
}
