//! Ticket #2864 / #2063 tail: rate_policy_save.rs uses checked arithmetic (try_minor)
//! for rate_policy_value_tiers upper_inclusive_thb (1 site).

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_kernel::money::Money;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const OWNER: &str = "owner";
const OWNER_ID: &str = "owner-1";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set");
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let schema = unique("k2063_rate_policy_save");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to admin");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to pool");
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
            .expect("insert branch");
        Self { pool, schema }
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop schema");
        pool.close().await;
    }
}

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: Some(Actor::owner(OWNER_ID)),
                staff: None,
            }
        })
    }
}

fn frame_guards() -> Value {
    json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        }
    })
}

#[test]
fn test_money_try_minor_catches_overflow_where_minor_wraps() {
    let overflow_minor = (i64::MAX as i128) + 1;
    let m = Money::try_from_minor_i128(overflow_minor).expect("valid decimal");
    // Unchecked .minor() wrapped silently past i64::MAX hundredths to negative:
    assert_eq!(m.minor(), i64::MIN);
    // Checked try_minor() returns an Err:
    assert!(m.try_minor().is_err());
}

#[actix_web::test]
async fn rate_policy_save_upper_inclusive_thb_overflow_returns_validation_failed() {
    let db = TestDb::start().await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    // 92233720368547758.08 THB = 9223372036854775808 hundredths = i64::MAX + 1 hundredths.
    let overflowing_amount = "92233720368547758.08";
    let body = json!({
        "branch": B01,
        "buyAdjustPct": "0",
        "sellAdjustPct": "0",
        "guards": frame_guards(),
        "maxReferenceAgeHours": 24,
        "onStale": "LIMIT",
        "staleLimitThb": "5000.00",
        "reason": "testing upper_inclusive_thb overflow",
        "valueTiersOn": false,
        "valueTiers": [
            {
                "tierNo": 1,
                "upperInclusiveThb": overflowing_amount,
                "buyMarginPct": "0",
                "sellMarginPct": "0"
            }
        ]
    });

    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri("/api/rate-policies")
        .insert_header((header::HOST, HOST))
        .insert_header((AS, OWNER))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string())
        .to_request();

    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {val}");
    assert_eq!(val["error"]["code"], "validationFailed", "body: {val}");
    assert_eq!(val["error"]["field"], "valueTiers", "body: {val}");

    db.finish().await;
}
