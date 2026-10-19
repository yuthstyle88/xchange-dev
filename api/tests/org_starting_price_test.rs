//! Ticket #1585 (SP-1) — the owner enters starting prices: command `orgStartingPriceSave`
//! and view `org_starting_prices`.
//!
//! Written before the implementation by test-writer gemini/pool-2, ordered by the BE lane
//! controller claude/ad05bf8d. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Held to the seven rows of issue #1585 §9 (and `docs/design/blueprint/rates.md` §12.10 SP-1):
//! 1. a non-OWNER saves: refused (`roleNotAllowedHere`), nothing written.
//! 2. OWNER saves, then changes a price: two versions; the first row is unchanged (immutable).
//! 3. a save: the mirrored `'START'` batch (`provider = 'START:'||org`) and one `'START'` line per currency, in the same transaction.
//! 4. an unpublished branch with an active policy: gets a publication with priced sides.
//! 5. an unpublished branch without a policy: no draft, no publication row; listed in the answer's `unpriced` with `noPolicy`.
//! 6. a published branch: untouched.
//! 7. buy > sell, or all-NULL entry: refused (the table's CHECKs).
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Each test works in a PostgreSQL schema of its own.

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
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";

const CMD_SAVE: &str = "/api/v3/commands/org_starting_price_save";
const VIEW_PRICES: &str = "/api/v3/views/org_starting_prices";

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
        let schema = unique("sp1startingprices");
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

    async fn insert_policy_version(&self, branch: &str, version: i32) -> Uuid {
        let policy_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO rate_policy_versions ( \
                id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
                approved_by, reason, source_version \
             ) VALUES ( \
                $1, $2, $3, 0.0000, 0.0000, \
                '{\"USD\": {\"buy\": {\"min_pct\": \"-1.0000\", \"max_pct\": \"1.0000\"}, \"sell\": {\"min_pct\": \"-1.0000\", \"max_pct\": \"1.0000\"}}}'::jsonb, \
                24, 'LIMIT', 5000.00, now() - interval '1 hour', NULL, \
                'owner-1', 'initial policy for test', NULL \
             )",
        )
        .bind(policy_id)
        .bind(branch)
        .bind(version)
        .execute(&self.pool)
        .await
        .expect("insert rate_policy_version");

        policy_id
    }

    async fn seed_publication(&self, branch: &str, version: i32) -> Uuid {
        let req_id = Uuid::new_v4();
        let pub_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();
        let idem_key = unique("pub_req");

        sqlx::query(
            "INSERT INTO approval_requests ( \
                id, type, branch_id, object_id, object_version_hash, policy_version, \
                proposed_payload, requested_by, requested_at, status, idempotency_key \
             ) VALUES ( \
                $1, 'RATE_PUBLICATION', $2, $3, 'seed_hash', '1', \
                '{}'::jsonb, 'owner', now(), 'APPROVED', $4 \
             )",
        )
        .bind(req_id)
        .bind(branch)
        .bind(pub_id.to_string())
        .bind(idem_key)
        .execute(&self.pool)
        .await
        .expect("insert approval_request");

        sqlx::query(
            "INSERT INTO approval_decisions ( \
                id, request_id, approver_id, approver_role, outcome, reason, at \
             ) VALUES ( \
                gen_random_uuid(), $1, 'owner', 'OWNER', 'APPROVED', 'seed publish', now() \
             )",
        )
        .bind(req_id)
        .execute(&self.pool)
        .await
        .expect("insert approval_decision");

        sqlx::query(
            "INSERT INTO rate_publications ( \
                id, branch_id, version, published_at, approved_request_id, published_by \
             ) VALUES ( \
                $1, $2, $3, now(), $4, 'owner' \
             )",
        )
        .bind(pub_id)
        .bind(branch)
        .bind(version)
        .bind(req_id)
        .execute(&self.pool)
        .await
        .expect("insert rate_publication");

        sqlx::query(
            "INSERT INTO published_rate_lines ( \
                id, publication_id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, source_snapshot \
             ) VALUES ( \
                $1, $2, 'USD', 'ALL', 1.0, 32.00000000, 33.00000000, '{\"provider\": \"SEED\"}'::jsonb \
             )",
        )
        .bind(line_id)
        .bind(pub_id)
        .execute(&self.pool)
        .await
        .expect("insert published_rate_line");

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(pub_id)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("update branch current_publication_id");

        pub_id
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
        "rates" | "RATES@XC-B01" => 3,
        "teller" | "TELLER@XC-B01" => 4,
        "branch_manager" | "BRANCH_MANAGER@XC-B01" => 5,
        other => panic!("no user id written for {other}"),
    };
    format!("00000000-0000-4000-8000-{n:012}")
}

fn user_of(spec: &str, org: &str) -> Option<AccessActor> {
    let (role, branch) = match spec {
        "owner" => ("EXECUTIVE", "*"),
        "admin" => ("ADMIN", "*"),
        "rates" => ("RATES", "XC-B01"),
        "teller" => ("TELLER", "XC-B01"),
        "branch_manager" => ("BRANCH_MANAGER", "XC-B01"),
        other => other.split_once('@')?,
    };
    let role = match role {
        "EXECUTIVE" => AccessRole::Executive,
        "ADMIN" => AccessRole::Admin,
        "RATES" | "RATE_MANAGER" => AccessRole::RateManager,
        "TELLER" => AccessRole::Teller,
        "BRANCH_MANAGER" => AccessRole::BranchManager,
        "FINANCE" => AccessRole::Finance,
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

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
    }
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers {
        org: db.org.clone(),
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

async fn assert_no_starting_price_artifacts(db: &TestDb, context: &str) {
    let prices_count: i64 = sqlx::query_scalar("SELECT count(*) FROM org_starting_prices")
        .fetch_one(&db.pool)
        .await
        .expect("count org_starting_prices");
    assert_eq!(
        prices_count, 0,
        "nothing written to org_starting_prices ({context})"
    );

    let batch_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM rate_reference_batches WHERE provider LIKE 'START:%'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count START batches");
    assert_eq!(
        batch_count, 0,
        "no START reference batch must be written ({context})"
    );

    let line_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM rate_reference_lines WHERE reference_type = 'START'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count START lines");
    assert_eq!(
        line_count, 0,
        "no START reference line must be written ({context})"
    );
}

// ------------------------------------------------------------ tests

/// Behaviour table row 1: a non-OWNER saves -> refused (`roleNotAllowedHere`), nothing written.
#[actix_web::test]
async fn a_non_owner_saves_refused_role_not_allowed_here_nothing_written() {
    let db = TestDb::start().await;

    let non_owners = ["rates", "admin", "branch_manager", "teller"];
    for who in non_owners {
        let payload = save_payload(
            vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
            &format!("attempt by non-owner {who}"),
        );

        let a = call(&db, who, Method::POST, CMD_SAVE, Some(payload)).await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "non-OWNER ({who}) save must be refused with 400: {}",
            a.text
        );
        assert_eq!(
            a.code(),
            "roleNotAllowedHere",
            "refusal for {who} must be roleNotAllowedHere: {}",
            a.text
        );

        assert_no_starting_price_artifacts(&db, &format!("attempt by non-owner {who}")).await;
    }

    db.finish().await;
}

/// Behaviour table row 2: OWNER saves, then changes a price -> two versions; the first row is unchanged (immutable).
#[actix_web::test]
async fn owner_saves_then_changes_a_price_two_versions_first_row_unchanged() {
    let db = TestDb::start().await;

    // First save: version 1
    let p1 = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "initial starting price",
    );
    let a1 = call(&db, "owner", Method::POST, CMD_SAVE, Some(p1)).await;
    assert_eq!(
        a1.status,
        StatusCode::OK,
        "first save must succeed: {}",
        a1.text
    );

    // Verify row 1 in DB
    let count1: i64 =
        sqlx::query_scalar("SELECT count(*) FROM org_starting_prices WHERE currency_code = 'USD'")
            .fetch_one(&db.pool)
            .await
            .expect("count USD starting prices");
    assert_eq!(count1, 1, "must have 1 USD starting price row");

    // Second save: price change creates version 2
    let p2 = save_payload(
        vec![entry("USD", Some("32.50000000"), Some("33.50000000"))],
        "updated starting price",
    );
    let a2 = call(&db, "owner", Method::POST, CMD_SAVE, Some(p2)).await;
    assert_eq!(
        a2.status,
        StatusCode::OK,
        "second save must succeed: {}",
        a2.text
    );

    // Verify two versions in DB, and version 1 is unchanged (immutable)
    let rows: Vec<(i32, Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT version, buy::text, sell::text, reason FROM org_starting_prices \
         WHERE currency_code = 'USD' ORDER BY version ASC",
    )
    .fetch_all(&db.pool)
    .await
    .expect("fetch USD starting price versions");

    assert_eq!(rows.len(), 2, "must have 2 versions in org_starting_prices");
    assert_eq!(rows[0].0, 1, "first row version is 1");
    assert_eq!(
        rows[0].1.as_deref(),
        Some("32.00000000"),
        "first row buy is unchanged"
    );
    assert_eq!(
        rows[0].2.as_deref(),
        Some("33.00000000"),
        "first row sell is unchanged"
    );
    assert_eq!(
        rows[0].3, "initial starting price",
        "first row reason is unchanged"
    );

    assert_eq!(rows[1].0, 2, "second row version is 2");
    assert_eq!(
        rows[1].1.as_deref(),
        Some("32.50000000"),
        "second row new buy"
    );
    assert_eq!(
        rows[1].2.as_deref(),
        Some("33.50000000"),
        "second row new sell"
    );
    assert_eq!(rows[1].3, "updated starting price", "second row reason");

    // View returns the starting prices and lists both versions
    let v = call(&db, "owner", Method::GET, VIEW_PRICES, None).await;
    assert_eq!(v.status, StatusCode::OK, "view must return 200: {}", v.text);

    let list = v.body["data"]
        .as_array()
        .expect("view must return data array (OrgViewAnswer { changed, versions, data })");

    let v1 = list.iter().any(|r| r["version"] == 1);
    let v2 = list.iter().any(|r| r["version"] == 2);
    assert!(v1, "view must list version 1: {}", v.text);
    assert!(v2, "view must list version 2: {}", v.text);

    db.finish().await;
}

/// Behaviour table row 3: a save -> mirrored 'START' batch (`provider = 'START:'||org`)
/// and one 'START' line per currency, in the same transaction.
#[actix_web::test]
async fn a_save_mirrored_start_batch_and_start_line_per_currency_in_same_tx() {
    let db = TestDb::start().await;

    let payload = save_payload(
        vec![
            entry("USD", Some("32.00000000"), Some("33.00000000")),
            entry("JPY", Some("0.22000000"), Some("0.23000000")),
        ],
        "initial starting rates",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(a.status, StatusCode::OK, "save must succeed: {}", a.text);

    let expected_provider = format!("START:{}", db.org);

    // Verify rate_reference_batches
    let batch: (Uuid, String, String) = sqlx::query_as(
        "SELECT id, provider, status FROM rate_reference_batches WHERE provider = $1",
    )
    .bind(&expected_provider)
    .fetch_one(&db.pool)
    .await
    .expect("mirrored START reference batch");
    assert_eq!(batch.1, expected_provider);
    assert_eq!(batch.2, "COMPLETE");

    #[derive(sqlx::FromRow)]
    struct RefLineRow {
        id: Uuid,
        currency_code: String,
        reference_type: String,
        unit_size: String,
        buy: Option<String>,
        sell: Option<String>,
    }

    // Verify rate_reference_lines: one per currency
    let lines: Vec<RefLineRow> = sqlx::query_as(
        "SELECT id, currency_code, reference_type, unit_size::text, buy::text, sell::text \
         FROM rate_reference_lines WHERE batch_id = $1 ORDER BY currency_code ASC",
    )
    .bind(batch.0)
    .fetch_all(&db.pool)
    .await
    .expect("START reference lines");

    assert_eq!(lines.len(), 2, "must write 2 reference lines");

    // JPY line: unit_size 100
    assert_eq!(lines[0].currency_code.trim(), "JPY");
    assert_eq!(lines[0].reference_type, "START");
    assert_eq!(lines[0].unit_size, "100.0000");
    assert_eq!(lines[0].buy.as_deref(), Some("0.22000000"));
    assert_eq!(lines[0].sell.as_deref(), Some("0.23000000"));

    // USD line: unit_size 1
    assert_eq!(lines[1].currency_code.trim(), "USD");
    assert_eq!(lines[1].reference_type, "START");
    assert_eq!(lines[1].unit_size, "1.0000");
    assert_eq!(lines[1].buy.as_deref(), Some("32.00000000"));
    assert_eq!(lines[1].sell.as_deref(), Some("33.00000000"));

    // Verify org_starting_prices links to the reference lines
    let prices: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT currency_code, reference_line_id FROM org_starting_prices \
         ORDER BY currency_code ASC",
    )
    .fetch_all(&db.pool)
    .await
    .expect("org_starting_prices rows");

    assert_eq!(prices.len(), 2);
    assert_eq!(prices[0].0.trim(), "JPY");
    assert_eq!(prices[0].1, lines[0].id, "JPY reference_line_id matches");
    assert_eq!(prices[1].0.trim(), "USD");
    assert_eq!(prices[1].1, lines[1].id, "USD reference_line_id matches");

    db.finish().await;
}

/// Behaviour table row 4: an unpublished branch with an active policy -> gets a publication with priced sides.
/// (Depends on SP-2 #1583 START fallback in ReferenceBase; stays red until both land).
#[actix_web::test]
async fn unpublished_branch_with_active_policy_gets_publication_with_priced_sides() {
    let db = TestDb::start().await;

    // Branch XC-B01 has active policy version, but current_publication_id IS NULL
    db.insert_policy_version(B01, 1).await;
    let initial_pub: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("check initial branch publication");
    assert!(
        initial_pub.is_none(),
        "XC-B01 must be unpublished initially"
    );

    let payload = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "pricing unpublished branch",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(a.status, StatusCode::OK, "save must succeed: {}", a.text);

    // XC-B01 must not be in unpriced
    if let Some(unpriced) = a.body.get("unpriced").and_then(|u| u.as_array()) {
        assert!(
            !unpriced.iter().any(|u| u["branch"].as_str() == Some(B01)),
            "XC-B01 has active policy and must be priced, not in unpriced: {unpriced:?}"
        );
    }

    // XC-B01 now has current_publication_id set
    let new_pub: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("check branch publication after save");
    let pub_id =
        new_pub.expect("XC-B01 must have a current_publication_id after starting price save");

    // The publication exists
    let pub_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM rate_publications WHERE id = $1 AND branch_id = $2",
    )
    .bind(pub_id)
    .bind(B01)
    .fetch_one(&db.pool)
    .await
    .expect("count publications");
    assert_eq!(pub_count, 1, "rate_publications row must exist");

    // The publication has published rate lines with priced sides (buy and sell not null)
    let priced_lines: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM published_rate_lines \
         WHERE publication_id = $1 AND currency_code = 'USD' \
           AND buy_rate IS NOT NULL AND sell_rate IS NOT NULL",
    )
    .bind(pub_id)
    .fetch_one(&db.pool)
    .await
    .expect("count priced published rate lines");
    assert!(priced_lines >= 1, "USD must have priced buy and sell sides");

    db.finish().await;
}

/// Behaviour table row 5: an unpublished branch without a policy -> no draft, no publication row;
/// listed in the answer's `unpriced` with `noPolicy`.
#[actix_web::test]
async fn unpublished_branch_without_policy_no_draft_no_publication_listed_in_unpriced_no_policy() {
    let db = TestDb::start().await;

    // XC-B02 has NO policy versions and current_publication_id IS NULL
    let policy_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_policy_versions WHERE branch_id = $1")
            .bind(B02)
            .fetch_one(&db.pool)
            .await
            .expect("check policy count");
    assert_eq!(policy_count, 0, "XC-B02 has no policy");

    let payload = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "pricing with unconfigured branch",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(a.status, StatusCode::OK, "save must succeed: {}", a.text);

    // Answer lists XC-B02 in unpriced with reason noPolicy
    let unpriced = a
        .body
        .get("unpriced")
        .and_then(|u| u.as_array())
        .expect("answer must contain unpriced array");
    let found = unpriced
        .iter()
        .any(|u| u["branch"].as_str() == Some(B02) && u["reason"].as_str() == Some("noPolicy"));
    assert!(
        found,
        "XC-B02 must be listed in unpriced with reason noPolicy, got: {unpriced:?}"
    );

    // XC-B02 still has NO current_publication_id
    let pub_id: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(B02)
            .fetch_one(&db.pool)
            .await
            .expect("check branch publication");
    assert!(
        pub_id.is_none(),
        "XC-B02 current_publication_id must remain null"
    );

    // XC-B02 has no publication rows
    let pub_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_publications WHERE branch_id = $1")
            .bind(B02)
            .fetch_one(&db.pool)
            .await
            .expect("count publications");
    assert_eq!(pub_count, 0, "no publication row for XC-B02");

    // XC-B02 has no drafts
    let draft_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_drafts WHERE branch_id = $1")
            .bind(B02)
            .fetch_one(&db.pool)
            .await
            .expect("count drafts");
    assert_eq!(draft_count, 0, "no draft row for XC-B02");

    db.finish().await;
}

/// Behaviour table row 6: a published branch -> untouched.
#[actix_web::test]
async fn published_branch_untouched() {
    let db = TestDb::start().await;

    // XC-B01 already has a publication and an active policy
    let initial_pub_id = db.seed_publication(B01, 1).await;
    db.insert_policy_version(B01, 1).await;

    let payload = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "starting price update",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(a.status, StatusCode::OK, "save must succeed: {}", a.text);

    // XC-B01 current_publication_id remains untouched
    let current_pub_id: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("check branch publication");
    assert_eq!(
        current_pub_id,
        Some(initial_pub_id),
        "published branch pointer must remain untouched"
    );

    // Publication count for XC-B01 remains 1 (no new publication created)
    let pub_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_publications WHERE branch_id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("count publications");
    assert_eq!(pub_count, 1, "no new publication for published branch");

    // XC-B01 has no new rate_drafts row
    let draft_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_drafts WHERE branch_id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("count drafts for XC-B01");
    assert_eq!(draft_count, 0, "XC-B01 must have no new rate_drafts row");

    // XC-B01 is not in unpriced
    if let Some(unpriced) = a.body.get("unpriced").and_then(|u| u.as_array()) {
        assert!(
            !unpriced.iter().any(|u| u["branch"].as_str() == Some(B01)),
            "XC-B01 is published and must not be in unpriced: {unpriced:?}"
        );
    }

    db.finish().await;
}

/// Behaviour table row 7: buy > sell, or all-NULL entry -> refused (the table's CHECKs).
#[actix_web::test]
async fn buy_greater_than_sell_or_all_null_entry_refused() {
    let db = TestDb::start().await;

    // Case 1: buy > sell
    let p_invalid_order = save_payload(
        vec![entry("USD", Some("35.00000000"), Some("30.00000000"))],
        "buy > sell attempt",
    );
    let a1 = call(&db, "owner", Method::POST, CMD_SAVE, Some(p_invalid_order)).await;
    assert_eq!(
        a1.status,
        StatusCode::BAD_REQUEST,
        "buy > sell must be refused with 400: {}",
        a1.text
    );
    assert_eq!(
        a1.code(),
        "validationFailed",
        "buy > sell refusal code must be validationFailed: {}",
        a1.text
    );
    assert_no_starting_price_artifacts(&db, "buy > sell refusal").await;

    // Case 2: all-NULL entry (both buy and sell are null)
    let p_all_null = save_payload(vec![entry("USD", None, None)], "all-null entry attempt");
    let a2 = call(&db, "owner", Method::POST, CMD_SAVE, Some(p_all_null)).await;
    assert_eq!(
        a2.status,
        StatusCode::BAD_REQUEST,
        "all-NULL entry must be refused with 400: {}",
        a2.text
    );
    assert_eq!(
        a2.code(),
        "validationFailed",
        "all-null refusal code must be validationFailed: {}",
        a2.text
    );
    assert_no_starting_price_artifacts(&db, "all-null refusal").await;

    // Case 3: multi-currency save where the second entry is invalid (first valid, second buy > sell)
    let p_second_invalid = save_payload(
        vec![
            entry("USD", Some("32.00000000"), Some("33.00000000")),
            entry("JPY", Some("0.25000000"), Some("0.20000000")),
        ],
        "second entry invalid attempt",
    );
    let a3 = call(&db, "owner", Method::POST, CMD_SAVE, Some(p_second_invalid)).await;
    assert_eq!(
        a3.status,
        StatusCode::BAD_REQUEST,
        "multi-currency save with invalid second entry must be refused with 400: {}",
        a3.text
    );
    assert_eq!(
        a3.code(),
        "validationFailed",
        "second entry invalid refusal code must be validationFailed: {}",
        a3.text
    );
    assert_no_starting_price_artifacts(&db, "second entry invalid refusal").await;

    db.finish().await;
}

/// Issue #1995 behaviour row (a): two unpublished branches, both with an active policy and
/// reference lines -> one draft per branch, with the full currency set on each branch.
#[actix_web::test]
async fn two_unpublished_branches_each_gets_full_currency_set() {
    let db = TestDb::start().await;

    db.insert_policy_version(B01, 1).await;
    db.insert_policy_version(B02, 1).await;

    let payload = save_payload(
        vec![
            entry("USD", Some("32.00000000"), Some("33.00000000")),
            entry("JPY", Some("0.22000000"), Some("0.23000000")),
        ],
        "pricing two unpublished branches",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(a.status, StatusCode::OK, "save must succeed: {}", a.text);

    if let Some(unpriced) = a.body.get("unpriced").and_then(|u| u.as_array()) {
        for branch in [B01, B02] {
            assert!(
                !unpriced
                    .iter()
                    .any(|u| u["branch"].as_str() == Some(branch)),
                "{branch} has an active policy and must not be in unpriced: {unpriced:?}"
            );
        }
    }

    for branch in [B01, B02] {
        let draft_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM rate_drafts WHERE branch_id = $1")
                .bind(branch)
                .fetch_one(&db.pool)
                .await
                .expect("count rate_drafts for branch");
        assert_eq!(draft_count, 1, "{branch} must have exactly one draft");

        let pub_id: Option<Uuid> =
            sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
                .bind(branch)
                .fetch_one(&db.pool)
                .await
                .expect("check branch current_publication_id");
        let pub_id = pub_id.unwrap_or_else(|| panic!("{branch} must have current_publication_id"));

        let priced_currencies: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT btrim(currency_code) AS code FROM published_rate_lines \
             WHERE publication_id = $1 AND buy_rate IS NOT NULL AND sell_rate IS NOT NULL \
             ORDER BY code ASC",
        )
        .bind(pub_id)
        .fetch_all(&db.pool)
        .await
        .expect("fetch published currencies for branch");

        assert_eq!(
            priced_currencies,
            vec!["JPY".to_string(), "USD".to_string()],
            "{branch} must receive the full currency set"
        );
    }

    db.finish().await;
}

/// Issue #1995 behaviour row (b): a currency code in `currencies` that fails `CurrencyCode::parse`
/// is skipped, while valid currencies are still priced.
#[actix_web::test]
async fn currency_code_failing_parse_is_skipped() {
    let db = TestDb::start().await;

    sqlx::query(
        "INSERT INTO currencies (code, name_th, name_en, symbol, unit_size) \
         VALUES ('ZZZ', 'สกุลเงินที่ไม่รู้จัก', 'Unknown Currency', 'Z', 1.0000)",
    )
    .execute(&db.pool)
    .await
    .expect("seed unparsable currency code ZZZ");

    db.insert_policy_version(B01, 1).await;

    let payload = save_payload(
        vec![
            entry("USD", Some("32.00000000"), Some("33.00000000")),
            entry("JPY", Some("0.22000000"), Some("0.23000000")),
        ],
        "pricing with unparsable currency code in currencies table",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "save must succeed and skip unparsable currency code ZZZ: {}",
        a.text
    );

    let pub_id: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("check XC-B01 current_publication_id");
    let pub_id = pub_id.expect("XC-B01 must have current_publication_id");

    let priced_currencies: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT btrim(currency_code) AS code FROM published_rate_lines \
         WHERE publication_id = $1 AND buy_rate IS NOT NULL AND sell_rate IS NOT NULL \
         ORDER BY code ASC",
    )
    .bind(pub_id)
    .fetch_all(&db.pool)
    .await
    .expect("fetch published currencies for XC-B01");

    assert_eq!(
        priced_currencies,
        vec!["JPY".to_string(), "USD".to_string()],
        "unparsable currency ZZZ must be skipped and valid currencies priced"
    );

    db.finish().await;
}

/// Issue #2398 row S1: `[USD 30/31, USD 30.5/31.5]` -> 400 `validationFailed` field `prices`;
/// `assert_no_starting_price_artifacts` holds.
#[actix_web::test]
async fn s1_duplicate_currency_usd_usd_refused_validation_failed_on_prices() {
    let db = TestDb::start().await;

    let payload = save_payload(
        vec![
            entry("USD", Some("30"), Some("31")),
            entry("USD", Some("30.5"), Some("31.5")),
        ],
        "duplicate USD",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "S1 duplicate USD+USD must answer 400: {} {}",
        a.status,
        a.text
    );
    assert_eq!(a.code(), "validationFailed", "{}", a.text);
    assert_eq!(a.body["error"]["field"], "prices", "{}", a.text);
    assert_no_starting_price_artifacts(&db, "S1 duplicate USD+USD").await;

    db.finish().await;
}

/// Issue #2398 row S2: `[USD 30/31, usd 30.5/31.5]` -> 400 `validationFailed` field `prices`;
/// nothing written.
#[actix_web::test]
async fn s2_duplicate_currency_case_insensitive_usd_refused_validation_failed_on_prices() {
    let db = TestDb::start().await;

    let payload = save_payload(
        vec![
            entry("USD", Some("30"), Some("31")),
            entry("usd", Some("30.5"), Some("31.5")),
        ],
        "duplicate USD+usd",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "S2 duplicate USD+usd must answer 400: {} {}",
        a.status,
        a.text
    );
    assert_eq!(a.code(), "validationFailed", "{}", a.text);
    assert_eq!(a.body["error"]["field"], "prices", "{}", a.text);
    assert_no_starting_price_artifacts(&db, "S2 duplicate USD+usd").await;

    db.finish().await;
}

/// Issue #2398 row S3: `[" USD" 30/31, "USD" 30.5/31.5]` -> 400 `validationFailed` field `prices`.
#[actix_web::test]
async fn s3_duplicate_currency_trimmed_usd_refused_validation_failed_on_prices() {
    let db = TestDb::start().await;

    let payload = save_payload(
        vec![
            entry(" USD", Some("30"), Some("31")),
            entry("USD", Some("30.5"), Some("31.5")),
        ],
        "duplicate trimmed USD",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "S3 duplicate ' USD'+'USD' must answer 400: {} {}",
        a.status,
        a.text
    );
    assert_eq!(a.code(), "validationFailed", "{}", a.text);
    assert_eq!(a.body["error"]["field"], "prices", "{}", a.text);
    assert_no_starting_price_artifacts(&db, "S3 duplicate trimmed USD").await;

    db.finish().await;
}

/// Issue #2398 row S4: `[USD 32/33, JPY 0.20/0.25]` (distinct) -> 200, saved.
#[actix_web::test]
async fn s4_distinct_currencies_usd_and_jpy_saved() {
    let db = TestDb::start().await;

    let payload = save_payload(
        vec![
            entry("USD", Some("32"), Some("33")),
            entry("JPY", Some("0.20"), Some("0.25")),
        ],
        "distinct USD and JPY",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "S4 distinct USD+JPY must answer 200: {} {}",
        a.status,
        a.text
    );
    let saved_count: i64 = sqlx::query_scalar("SELECT count(*) FROM org_starting_prices")
        .fetch_one(&db.pool)
        .await
        .expect("count org_starting_prices");
    assert_eq!(saved_count, 2, "both USD and JPY must be saved");

    db.finish().await;
}
