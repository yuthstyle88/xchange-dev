//! Ticket #2339 — `rateDraftEdit` answers an unknown draft 404 `notFound` (matching its siblings),
//! and `rateDraftCreate` / `rateDraftEdit` refuse a repeated currency in `lines` with 400
//! `validationFailed` before any write.
//!
//! Behaviour table (§9):
//! 1. `PUT /api/rate-drafts/{random uuid}` → 404 `notFound`; nothing written
//! 2. `PUT /api/rate-drafts/{id}` where the draft exists but in another branch → 404 `notFound`;
//!    that draft unchanged
//! 3. `POST /api/rate-drafts` with `[USD AUTO, USD AUTO]` → 400 `validationFailed`; `rate_drafts`
//!    and `rate_draft_lines` row counts unchanged
//! 4. `POST` with `[USD AUTO, USD MANUAL 33/34 + reason]` (same currency, different modes) →
//!    400 `validationFailed`
//! 5. `PUT` on an existing 1-line draft with `[USD, USD]` → 400 `validationFailed`; the draft's
//!    `version` and its lines unchanged
//! 6. `POST` with `[USD, EUR]` (distinct) → 200 as today (keep)
//! 7. `PUT` with a correct id, the right `expectedVersion`, distinct lines → 200, version +1 (keep)
//! 8. `PUT` with a wrong `expectedVersion` → 409 `staleVersion` (keep)

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
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";
const RATES: &str = "staff:RATES:XC-B01";
const RATES_B02: &str = "staff:RATES:XC-B02";
const OWNER: &str = "owner";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
                "TEST_DATABASE_URL is not set.\n\
                 The rate draft input refusal tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("v3ratedraftrefusal");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
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
            .expect("apply api/migrations to the test schema");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        Self { pool, schema }
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    async fn seed_usd(&self) {
        let batch: String = sqlx::query_scalar(
            "INSERT INTO rate_reference_batches \
               (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES (gen_random_uuid(), 'BOT', (now() - interval '1 hour')::date, \
                     now() - interval '1 hour', $1, 'COMPLETE') \
             RETURNING id::text",
        )
        .bind(unique("checksum"))
        .fetch_one(&self.pool)
        .await
        .expect("insert into rate_reference_batches (§4.1)");
        sqlx::query(
            "INSERT INTO rate_reference_lines \
               (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
             VALUES (gen_random_uuid(), $1::uuid, 'USD', 'BOT', 1, 32.1, 32.45, '{}'::jsonb), \
                    (gen_random_uuid(), $1::uuid, 'EUR', 'BOT', 1, 36.1, 36.50, '{}'::jsonb)",
        )
        .bind(&batch)
        .execute(&self.pool)
        .await
        .expect("insert into rate_reference_lines (§4.1)");
        let guard = json!({ "min_pct": "-1.0000", "max_pct": "1.0000" });
        sqlx::query(
            "INSERT INTO rate_policy_versions \
               (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
                approved_by, reason, source_version) \
             VALUES (gen_random_uuid(), $1, 1, 0, 0, $2, NULL, NULL, NULL, \
                     now() - interval '1 day', NULL, 'owner-1', 'seeded by the test', NULL)",
        )
        .bind(B01)
        .bind(json!({ "USD": { "buy": guard.clone(), "sell": guard } }))
        .execute(&self.pool)
        .await
        .expect("insert into rate_policy_versions (§4.1)");
    }

    async fn draft_snapshot(&self, id: &str) -> (String, String) {
        let row: String = sqlx::query_scalar(
            "SELECT row_to_json(d)::text FROM rate_drafts d WHERE d.id::text = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read draft {id}: {e}"));
        let lines: String = sqlx::query_scalar(
            "SELECT coalesce(string_agg(row_to_json(l)::text, '|' ORDER BY l.id), '') \
             FROM rate_draft_lines l WHERE l.draft_id::text = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read the lines of draft {id}: {e}"));
        (row, lines)
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

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    if spec == OWNER {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff(
            format!("employee-{}", role.as_str()),
            role,
            BranchId::new(branch),
        ),
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

async fn command(db: &TestDb, who: &str, method: Method, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
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
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn create_draft(db: &TestDb, who: &str, lines: Vec<Value>) -> String {
    let a = command(
        db,
        who,
        Method::POST,
        "/api/rate-drafts",
        json!({ "branch": B01, "lines": lines }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "create a draft: {}", a.text);
    assert_eq!(a.body["status"], json!("DRAFT"), "a new draft: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the draft's id: {}", a.text))
        .to_owned()
}

fn auto_usd() -> Value {
    json!({
        "currency": "USD",
        "buyMode": "AUTO",
        "sellMode": "AUTO",
        "manualBuy": null,
        "manualSell": null,
        "reason": null,
    })
}

fn auto_eur() -> Value {
    json!({
        "currency": "EUR",
        "buyMode": "AUTO",
        "sellMode": "AUTO",
        "manualBuy": null,
        "manualSell": null,
        "reason": null,
    })
}

fn manual_usd_33_34() -> Value {
    json!({
        "currency": "USD",
        "buyMode": "MANUAL",
        "sellMode": "MANUAL",
        "manualBuy": "33.00",
        "manualSell": "34.00",
        "reason": "manual override reason",
    })
}

// ------------------------------------------------------------ tests (rows 1–8)

/// Row 1: `PUT /api/rate-drafts/{random uuid}` → 404 `notFound`; nothing written.
#[actix_web::test]
async fn row1_put_unknown_draft_id_answers_404_not_found_and_writes_nothing() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let unknown_id = Uuid::new_v4();

    let ans = command(
        &db,
        RATES,
        Method::PUT,
        &format!("/api/rate-drafts/{unknown_id}"),
        json!({
            "branch": B01,
            "expectedVersion": 1,
            "lines": [auto_usd()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "row 1: unknown draft on PUT must answer 404 notFound: {}",
        ans.text
    );
    assert_eq!(ans.code(), "notFound", "row 1: {}", ans.text);
    assert_eq!(
        db.count("rate_drafts").await,
        0,
        "row 1: no rate_drafts row written"
    );
    assert_eq!(
        db.count("rate_draft_lines").await,
        0,
        "row 1: no rate_draft_lines row written"
    );
    db.finish().await;
}

/// Row 2: `PUT /api/rate-drafts/{id}` where the draft exists but in another branch →
/// 404 `notFound` (no oracle for other branches); that draft unchanged.
#[actix_web::test]
async fn row2_put_draft_from_another_branch_answers_404_not_found_and_leaves_draft_unchanged() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![auto_usd()]).await;
    let snapshot_before = db.draft_snapshot(&id).await;

    let ans = command(
        &db,
        RATES_B02,
        Method::PUT,
        &format!("/api/rate-drafts/{id}"),
        json!({
            "branch": B02,
            "expectedVersion": 1,
            "lines": [auto_eur()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "row 2: draft belonging to another branch must answer 404 notFound: {}",
        ans.text
    );
    assert_eq!(ans.code(), "notFound", "row 2: {}", ans.text);
    assert_eq!(
        db.draft_snapshot(&id).await,
        snapshot_before,
        "row 2: the draft in XC-B01 must remain completely unchanged"
    );
    db.finish().await;
}

/// Row 3: `POST /api/rate-drafts` with `[USD AUTO, USD AUTO]` → 400 `validationFailed`;
/// `rate_drafts` and `rate_draft_lines` row counts unchanged.
#[actix_web::test]
async fn row3_post_duplicate_currency_auto_answers_400_validation_failed_and_writes_nothing() {
    let db = TestDb::start().await;
    db.seed_usd().await;

    let ans = command(
        &db,
        RATES,
        Method::POST,
        "/api/rate-drafts",
        json!({
            "branch": B01,
            "lines": [auto_usd(), auto_usd()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "row 3: duplicate USD AUTO on POST must answer 400 validationFailed: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "row 3: {}", ans.text);
    assert_eq!(
        db.count("rate_drafts").await,
        0,
        "row 3: rate_drafts row count must remain 0"
    );
    assert_eq!(
        db.count("rate_draft_lines").await,
        0,
        "row 3: rate_draft_lines row count must remain 0"
    );
    db.finish().await;
}

/// Row 4: `POST` with `[USD AUTO, USD MANUAL 33/34 + reason]` (same currency, different modes) →
/// 400 `validationFailed`.
#[actix_web::test]
async fn row4_post_duplicate_currency_different_modes_answers_400_validation_failed() {
    let db = TestDb::start().await;
    db.seed_usd().await;

    let ans = command(
        &db,
        RATES,
        Method::POST,
        "/api/rate-drafts",
        json!({
            "branch": B01,
            "lines": [auto_usd(), manual_usd_33_34()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "row 4: duplicate USD with AUTO and MANUAL modes on POST must answer 400 validationFailed: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "row 4: {}", ans.text);
    assert_eq!(
        db.count("rate_drafts").await,
        0,
        "row 4: rate_drafts row count must remain 0"
    );
    assert_eq!(
        db.count("rate_draft_lines").await,
        0,
        "row 4: rate_draft_lines row count must remain 0"
    );
    db.finish().await;
}

/// Row 5: `PUT` on an existing 1-line draft with `[USD, USD]` → 400 `validationFailed`;
/// the draft's `version` and its lines unchanged.
#[actix_web::test]
async fn row5_put_duplicate_currency_answers_400_validation_failed_and_leaves_draft_unchanged() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![auto_usd()]).await;
    let snapshot_before = db.draft_snapshot(&id).await;

    let ans = command(
        &db,
        RATES,
        Method::PUT,
        &format!("/api/rate-drafts/{id}"),
        json!({
            "branch": B01,
            "expectedVersion": 1,
            "lines": [auto_usd(), manual_usd_33_34()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "row 5: duplicate USD on PUT must answer 400 validationFailed: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "row 5: {}", ans.text);
    assert_eq!(
        db.draft_snapshot(&id).await,
        snapshot_before,
        "row 5: the draft's version and its lines must remain unchanged"
    );
    db.finish().await;
}

/// Row 6: `POST` with `[USD, EUR]` (distinct) → 200 as today (keep).
#[actix_web::test]
async fn row6_post_distinct_currencies_answers_200_ok() {
    let db = TestDb::start().await;
    db.seed_usd().await;

    let ans = command(
        &db,
        RATES,
        Method::POST,
        "/api/rate-drafts",
        json!({
            "branch": B01,
            "lines": [auto_usd(), auto_eur()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "row 6: distinct currencies [USD, EUR] on POST must answer 200: {}",
        ans.text
    );
    assert_eq!(ans.body["version"], json!(1), "row 6: {}", ans.text);
    assert_eq!(ans.body["status"], json!("DRAFT"), "row 6: {}", ans.text);
    assert_eq!(db.count("rate_drafts").await, 1);
    assert_eq!(db.count("rate_draft_lines").await, 2);
    db.finish().await;
}

/// Row 7: `PUT` with a correct id, the right `expectedVersion`, distinct lines →
/// 200, version +1 as today (keep).
#[actix_web::test]
async fn row7_put_distinct_currencies_with_matching_version_answers_200_and_increments_version() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![auto_usd()]).await;

    let ans = command(
        &db,
        RATES,
        Method::PUT,
        &format!("/api/rate-drafts/{id}"),
        json!({
            "branch": B01,
            "expectedVersion": 1,
            "lines": [auto_usd(), auto_eur()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "row 7: valid PUT with distinct lines must answer 200: {}",
        ans.text
    );
    assert_eq!(ans.body["id"], json!(id), "row 7: {}", ans.text);
    assert_eq!(ans.body["version"], json!(2), "row 7: {}", ans.text);
    assert_eq!(ans.body["status"], json!("DRAFT"), "row 7: {}", ans.text);
    assert_eq!(db.count("rate_draft_lines").await, 2);
    db.finish().await;
}

/// Row 8: `PUT` with a wrong `expectedVersion` → 409 `staleVersion` as today (keep).
#[actix_web::test]
async fn row8_put_wrong_expected_version_answers_409_stale_version() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![auto_usd()]).await;
    let snapshot_before = db.draft_snapshot(&id).await;

    let ans = command(
        &db,
        RATES,
        Method::PUT,
        &format!("/api/rate-drafts/{id}"),
        json!({
            "branch": B01,
            "expectedVersion": 99,
            "lines": [auto_usd()],
        }),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::CONFLICT,
        "row 8: wrong expectedVersion on PUT must answer 409 staleVersion: {}",
        ans.text
    );
    assert_eq!(ans.code(), "staleVersion", "row 8: {}", ans.text);
    assert_eq!(
        db.draft_snapshot(&id).await,
        snapshot_before,
        "row 8: draft must remain unchanged on staleVersion"
    );
    db.finish().await;
}
