//! R-planner (#2054 row 6, design `docs/design/blueprint/bounded-inputs.md` §4 R-*) up-front tests — written before
//! the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller claude/56a74fcd. Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! `plannerDraftSave` (`POST /api/app/planner-drafts`, anonymous app sessions included) bounds `amount.quantity` at
//! the boundary: positive, at most `Money::MAX_INPUT_UNITS` (10^8, inclusive, the cap of `ForeignAmount::parse`), at
//! most 4 decimals (`planner_drafts.quantity numeric(20,4) CHECK (> 0)`, `customer_app.sql:15`). A refusal is
//! **400 `validationFailed` on `quantity`** and no draft is written. Both sides of the cap are pinned.
//!
//! Harness copied from `planner_draft_other_org_branch_test.rs`. Needs `TEST_DATABASE_URL` (`_test`); nothing skips.

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

use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const MAIN: &str = "MAIN";
const YY: &str = "YY";
const YY_ID: &str = "00000000-0000-0000-0000-0000000000a2";
const OTHER_BRANCH: &str = "ZZP6-B1";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("r_planner_qty");
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

        common::install_test_pii_keys();

        // Seed the second organisation YY and its branch OTHER_BRANCH (as in app_public_reads_test.rs:142-149)
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ('{YY_ID}', '{YY}', NULL, 'Asia/Bangkok', now())"
        )))
        .execute(&pool)
        .await
        .expect("insert second organization");
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO branches (id, name, organization_id, code) \
             VALUES ('{OTHER_BRANCH}', 'Other {OTHER_BRANCH}', '{YY_ID}', '{OTHER_BRANCH}')"
        )))
        .execute(&pool)
        .await
        .expect("insert branch of second organization");

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

struct AnonCallers;

impl SessionReader for AnonCallers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            Callers {
                owner: None,
                staff: None,
                user: None,
                staff_user: None,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }
}

async fn call(
    room: &Room,
    cookie: Option<&str>,
    method: Method,
    uri: &str,
    body: Option<(Value, &str)>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(AnonCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"));

    if let Some(c) = cookie {
        req = req.insert_header((header::COOKIE, format!("{APP_SESSION_COOKIE}={c}")));
    }
    if let Some((b, key)) = body {
        req = req
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(b.to_string());
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

async fn drafts(room: &Room) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM planner_drafts")
        .fetch_one(&room.pool)
        .await
        .expect("count planner_drafts")
}

async fn save(room: &Room, quantity: &str) -> Answer {
    let session = AppSessionId::generate();
    let body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": quantity },
        "expectedVersion": null,
    });
    call(
        room,
        Some(session.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body, &unique("key"))),
    )
    .await
}

async fn accepted(room: &Room, quantity: &str) {
    let before = drafts(room).await;
    let a = save(room, quantity).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{quantity:?} must be saved: {}",
        a.text
    );
    assert_eq!(
        drafts(room).await,
        before + 1,
        "{quantity:?}: one draft written"
    );
}

async fn refused(room: &Room, quantity: &str) {
    let before = drafts(room).await;
    let a = save(room, quantity).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::BAD_REQUEST, "validationFailed".to_owned()),
        "{quantity:?}: body {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"].as_str(),
        Some("quantity"),
        "{quantity:?}: field, body {}",
        a.text
    );
    assert_eq!(drafts(room).await, before, "{quantity:?}: nothing written");
}

/// Row 1. The cap is inclusive at 10^8: the largest accepted next to the smallest refused.
#[actix_web::test]
async fn quantity_cap_is_inclusive_at_one_hundred_million() {
    let room = Room::start().await;
    accepted(&room, "100000000").await;
    accepted(&room, "100000000.0000").await;
    refused(&room, "100000000.01").await;
    refused(&room, "100000001").await;
    room.finish().await;
}

/// Row 2. A quantity past the column (`numeric(20,4)`, < 10^16) is 400, never 500.
#[actix_web::test]
async fn a_quantity_past_the_column_is_400_not_500() {
    let room = Room::start().await;
    refused(&room, "10000000000000000").await;
    refused(&room, &"9".repeat(40)).await;
    room.finish().await;
}

/// Row 3. A fifth non-zero decimal would be rounded by the column (to 0 here, then break `CHECK (> 0)`): refused.
#[actix_web::test]
async fn a_fifth_decimal_is_refused_not_rounded() {
    let room = Room::start().await;
    refused(&room, "0.00001").await;
    refused(&room, "1.00001").await;
    accepted(&room, "1.00000").await;
    room.finish().await;
}

/// Row 4 (kept). Zero, negatives and non-numbers stay refused on `quantity`.
#[actix_web::test]
async fn zero_negative_and_non_numbers_stay_refused() {
    let room = Room::start().await;
    for q in ["0", "0.0000", "-1", "abc", ""] {
        refused(&room, q).await;
    }
    room.finish().await;
}
