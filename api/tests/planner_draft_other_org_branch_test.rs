//! Ticket #2401: `plannerDraftSave` update naming another organisation's branch refuses with
//! 403 `outOfScope` instead of 500 `serverError`, leaving the draft unchanged.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

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
        let schema = unique("planner_other_org");
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

async fn create_main_draft(room: &Room, session: &AppSessionId, key: &str) -> String {
    let create_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });
    let res = call(
        room,
        Some(session.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((create_body, key)),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "create draft succeeded: {}",
        res.text
    );
    assert_eq!(res.body["version"].as_i64(), Some(1));
    assert_eq!(res.body["branch"].as_str(), Some(MAIN));
    res.body["id"].as_str().expect("draft id string").to_owned()
}

// P1: draft created at MAIN (org XC); update names a branch of another organisation,
// expectedVersion: 1 -> 403 outOfScope; draft still version 1 with branch MAIN.
#[actix_web::test]
async fn p1_update_naming_other_org_branch_returns_403_out_of_scope_and_keeps_draft() {
    let room = Room::start().await;
    let session = AppSessionId::generate();
    let draft_id = create_main_draft(&room, &session, "p1-create").await;

    let update_body = json!({
        "branch": OTHER_BRANCH,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "600.0000" },
        "expectedVersion": 1,
    });
    let res = call(
        &room,
        Some(session.as_str()),
        Method::PUT,
        &format!("/api/app/planner-drafts/{draft_id}"),
        Some((update_body, "p1-update-other-org")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::FORBIDDEN,
        "update naming another org's branch must return 403: {}",
        res.text
    );
    assert_eq!(res.code(), "outOfScope");

    // Verify draft is unchanged: version 1, branch MAIN, quantity 500.0000
    let view_res = call(
        &room,
        Some(session.as_str()),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(view_res.status, StatusCode::OK);
    let drafts = view_res.body["data"]["drafts"]
        .as_array()
        .or_else(|| view_res.body["data"].as_array())
        .expect("drafts array");
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0]["id"].as_str(), Some(draft_id.as_str()));
    assert_eq!(drafts[0]["version"].as_i64(), Some(1));
    assert_eq!(drafts[0]["branch"].as_str(), Some(MAIN));
    assert_eq!(drafts[0]["amount"]["quantity"].as_str(), Some("500.0000"));

    room.finish().await;
}

// P2: the same update with a stale expectedVersion: 5 -> 403 outOfScope
// (the organisation check runs before the version check).
#[actix_web::test]
async fn p2_update_naming_other_org_branch_with_stale_version_returns_403_out_of_scope() {
    let room = Room::start().await;
    let session = AppSessionId::generate();
    let draft_id = create_main_draft(&room, &session, "p2-create").await;

    let update_body = json!({
        "branch": OTHER_BRANCH,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "600.0000" },
        "expectedVersion": 5,
    });
    let res = call(
        &room,
        Some(session.as_str()),
        Method::PUT,
        &format!("/api/app/planner-drafts/{draft_id}"),
        Some((update_body, "p2-update-stale-other-org")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::FORBIDDEN,
        "org check runs before expectedVersion check and returns 403: {}",
        res.text
    );
    assert_eq!(res.code(), "outOfScope");

    room.finish().await;
}

// P3: update naming MAIN again, expectedVersion: 1 -> 200, version 2 (keep).
#[actix_web::test]
async fn p3_update_naming_main_with_expected_version_1_succeeds_at_version_2() {
    let room = Room::start().await;
    let session = AppSessionId::generate();
    let draft_id = create_main_draft(&room, &session, "p3-create").await;

    let update_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "600.0000" },
        "expectedVersion": 1,
    });
    let res = call(
        &room,
        Some(session.as_str()),
        Method::PUT,
        &format!("/api/app/planner-drafts/{draft_id}"),
        Some((update_body, "p3-update-main")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "update naming MAIN succeeded: {}",
        res.text
    );
    assert_eq!(res.body["version"].as_i64(), Some(2));
    assert_eq!(res.body["branch"].as_str(), Some(MAIN));

    room.finish().await;
}

// P4: another app session updates the draft, naming MAIN -> 403 outOfScope (keep: owned).
#[actix_web::test]
async fn p4_another_session_updating_draft_naming_main_returns_403_out_of_scope() {
    let room = Room::start().await;
    let session_a = AppSessionId::generate();
    let session_b = AppSessionId::generate();
    let draft_id = create_main_draft(&room, &session_a, "p4-create").await;

    let update_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "600.0000" },
        "expectedVersion": 1,
    });
    let res = call(
        &room,
        Some(session_b.as_str()),
        Method::PUT,
        &format!("/api/app/planner-drafts/{draft_id}"),
        Some((update_body, "p4-update-other-session")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::FORBIDDEN,
        "another session updating draft must return 403: {}",
        res.text
    );
    assert_eq!(res.code(), "outOfScope");

    room.finish().await;
}
