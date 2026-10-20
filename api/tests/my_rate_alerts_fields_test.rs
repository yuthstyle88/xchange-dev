//! Up-front specification tests for 108-Plaza/xchange#1609:
//! `my_rate_alerts` carries branch, unitSize, currentRate, ratePublishedAt.
//!
//! Behaviour table rows 1-8 from BE lane controller ruling issuecomment-6141092915,
//! with review fixes from issuecomment-6142030409 (positive controls on rows 4-7,
//! and non-current publication on row 4).
//! Written before implementation (gemini/studio-12, ordered by claude/ad05bf8d, Rule A).
//! Do not edit, delete or weaken.
//!
//! Needs TEST_DATABASE_URL naming a database whose name ends in `_test`.

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
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const CX1: &str = "CX-1609-1";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn dec_str(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("invalid decimal string '{s}': {e}"))
}

fn assert_published_at_eq(actual_val: &Value, expected: DateTime<Utc>, msg: &str) {
    let actual_str = actual_val
        .as_str()
        .unwrap_or_else(|| panic!("{msg}: expected string timestamp, got {actual_val}"));
    let actual_dt = chrono::DateTime::parse_from_rfc3339(actual_str)
        .unwrap_or_else(|e| panic!("{msg}: invalid RFC 3339 timestamp '{actual_str}': {e}"))
        .with_timezone(&Utc);
    assert_eq!(actual_dt.timestamp(), expected.timestamp(), "{msg}");
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
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
        let schema = unique("mraf");
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

        let room = Self { pool, schema };
        room.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{CX1}', '{ORG}', '{CX1}', 'Somchai', 'LIVE', now())"
        ))
        .await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn create_branch(&self, branch: &str) {
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) \
             VALUES ($1, $1, $2::uuid) ON CONFLICT (id) DO NOTHING",
        )
        .bind(branch)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("create branch");
    }

    async fn user(&self) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("u"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }

    async fn customer(&self, customer_id: &str) -> Who {
        let id = self.user().await;
        Who(format!("{id}|CUSTOMER|{customer_id}"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish_rate(
        &self,
        branch: &str,
        currency: &str,
        unit_size: Decimal,
        buy_rate: Option<Decimal>,
        sell_rate: Option<Decimal>,
        denom: &str,
        published_at: DateTime<Utc>,
    ) -> (uuid::Uuid, DateTime<Utc>) {
        let approval: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
             RETURNING id",
        )
        .bind(branch)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("approval request");

        let version: i32 = sqlx::query_scalar(
            "SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1",
        )
        .bind(branch)
        .fetch_one(&self.pool)
        .await
        .expect("version");

        let publication: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, 'test') RETURNING id",
        )
        .bind(branch)
        .bind(version)
        .bind(published_at)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("the publication");

        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, \
             source_snapshot) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(publication)
        .bind(currency)
        .bind(denom)
        .bind(unit_size)
        .bind(buy_rate)
        .bind(sell_rate)
        .bind(json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }))
        .execute(&self.pool)
        .await
        .expect("the rate line");

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("make it current");

        (publication, published_at)
    }

    async fn insert_alert(
        &self,
        customer_id: &str,
        branch: &str,
        currency: &str,
        shop_side: &str,
        unit_size: Decimal,
        target: Decimal,
    ) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        let key = unique("key");
        sqlx::query(
            "INSERT INTO rate_alerts (id, organization_id, customer_id, branch_id, currency_code, shop_side, \
             unit_size, condition, target, enabled, cooldown_seconds, idempotency_key, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, 'ABOVE', $8, true, 86400, $9, now(), now())",
        )
        .bind(id)
        .bind(ORG)
        .bind(customer_id)
        .bind(branch)
        .bind(currency)
        .bind(shop_side)
        .bind(unit_size)
        .bind(target)
        .bind(key)
        .execute(&self.pool)
        .await
        .expect("insert rate alert");
        id
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish_rate_with_snapshot(
        &self,
        branch: &str,
        currency: &str,
        unit_size: Decimal,
        buy_rate: Option<Decimal>,
        sell_rate: Option<Decimal>,
        denom: &str,
        published_at: DateTime<Utc>,
        snapshot: Value,
    ) -> (uuid::Uuid, uuid::Uuid) {
        let approval: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
             RETURNING id",
        )
        .bind(branch)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("approval request");

        let version: i32 = sqlx::query_scalar(
            "SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1",
        )
        .bind(branch)
        .fetch_one(&self.pool)
        .await
        .expect("version");

        let publication: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, 'test') RETURNING id",
        )
        .bind(branch)
        .bind(version)
        .bind(published_at)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("the publication");

        let line_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, \
             source_snapshot) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7) RETURNING id",
        )
        .bind(publication)
        .bind(currency)
        .bind(denom)
        .bind(unit_size)
        .bind(buy_rate)
        .bind(sell_rate)
        .bind(snapshot)
        .fetch_one(&self.pool)
        .await
        .expect("the rate line");

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("make it current");

        (publication, line_id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_alert_full(
        &self,
        customer_id: &str,
        branch: &str,
        currency: &str,
        shop_side: &str,
        unit_size: Decimal,
        target: Decimal,
        relation: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        let key = unique("key");
        sqlx::query(
            "INSERT INTO rate_alerts (id, organization_id, customer_id, branch_id, currency_code, shop_side, \
             unit_size, condition, target, enabled, cooldown_seconds, relation, expires_at, idempotency_key, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, 'ABOVE', $8, true, 86400, $9, $10, $11, now(), now())",
        )
        .bind(id)
        .bind(ORG)
        .bind(customer_id)
        .bind(branch)
        .bind(currency)
        .bind(shop_side)
        .bind(unit_size)
        .bind(target)
        .bind(relation)
        .bind(expires_at)
        .bind(key)
        .execute(&self.pool)
        .await
        .expect("insert rate alert");
        id
    }

    async fn insert_event(
        &self,
        alert_id: uuid::Uuid,
        line_id: uuid::Uuid,
        observed_rate: Decimal,
        trigger_at: DateTime<Utc>,
        episode_no: i32,
    ) {
        sqlx::query(
            "INSERT INTO rate_alert_events (id, organization_id, alert_id, published_rate_line_id, observed_rate, \
             trigger_at, episode_no, created_at) VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, $5, $6, now())",
        )
        .bind(ORG)
        .bind(alert_id)
        .bind(line_id)
        .bind(observed_rate)
        .bind(trigger_at)
        .bind(episode_no)
        .execute(&self.pool)
        .await
        .expect("insert rate alert event");
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ authentication

struct Who(String);

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, customer) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, None::<BranchId>, from, None)],
        (!customer.is_empty()).then_some(customer),
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

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

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn ok(&self, what: &str) -> Value {
        assert_eq!(self.status, StatusCode::OK, "{what}: {}", self.text);
        self.body.clone()
    }
}

async fn call(room: &Room, who: &Who, method: Method, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
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
        .insert_header((AS, who.0.clone()));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn my_alerts(room: &Room, who: &Who) -> Vec<Value> {
    let a = call(room, who, Method::GET, "/api/v3/views/my_rate_alerts").await;
    let body = a.ok("my_rate_alerts");
    body["data"]["alerts"]
        .as_array()
        .unwrap_or_else(|| panic!("my_rate_alerts answers data.alerts: {}", a.text))
        .clone()
}

// ------------------------------------------------------------ Behaviour table tests (Rows 1-8)

/// 1. An alert on branch B for JPY with unit_size 100 → branch == "B", unitSize == "100.0000".
#[actix_web::test]
async fn row1_alert_branch_and_unit_size() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;
    let alert_id = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 1, "expected 1 alert");
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(item["branch"], "B", "row 1: branch must be B");
    assert_eq!(
        item["unitSize"], "100.0000",
        "row 1: unitSize must be 100.0000"
    );

    room.finish().await;
}

/// 2. B's current publication has a JPY ALL line; the alert's shop_side = SELL
///    → currentRate == sell_rate.to_string(), ratePublishedAt == that publication's published_at (RFC 3339).
#[actix_web::test]
async fn row2_current_publication_jpy_all_sell_side() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;
    let pub_time = Utc.with_ymd_and_hms(2026, 10, 13, 8, 30, 0).unwrap();
    room.publish_rate(
        "B",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.23000000")),
        Some(dec_str("0.24000000")),
        "ALL",
        pub_time,
    )
    .await;
    let alert_id = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 1, "expected 1 alert");
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(
        item["currentRate"], "0.24000000",
        "row 2: currentRate must match publication sell_rate"
    );
    assert_published_at_eq(
        &item["ratePublishedAt"],
        pub_time,
        "row 2: ratePublishedAt must match publication published_at",
    );
    assert_eq!(item["branch"], "B", "row 2: branch must be B");
    assert_eq!(
        item["unitSize"], "100.0000",
        "row 2: unitSize must be 100.0000"
    );

    room.finish().await;
}

/// 3. The same, with shop_side = BUY → currentRate == buy_rate.to_string().
#[actix_web::test]
async fn row3_current_publication_jpy_all_buy_side() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;
    let pub_time = Utc.with_ymd_and_hms(2026, 10, 13, 8, 30, 0).unwrap();
    room.publish_rate(
        "B",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.23000000")),
        Some(dec_str("0.24000000")),
        "ALL",
        pub_time,
    )
    .await;
    let alert_id = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "BUY",
            dec_str("100"),
            dec_str("0.23000000"),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 1, "expected 1 alert");
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(
        item["currentRate"], "0.23000000",
        "row 3: currentRate must match publication buy_rate"
    );
    assert_published_at_eq(
        &item["ratePublishedAt"],
        pub_time,
        "row 3: ratePublishedAt must match publication published_at",
    );
    assert_eq!(item["branch"], "B", "row 3: branch must be B");
    assert_eq!(
        item["unitSize"], "100.0000",
        "row 3: unitSize must be 100.0000"
    );

    room.finish().await;
}

/// 4. B has no current publication → both null.
///    Positive control: branch D has a current JPY ALL line and an alert on D.
///    Must-fix 2: B has a publication, but current_publication_id is NULL.
#[actix_web::test]
async fn row4_branch_has_no_current_publication_both_null() {
    let room = Room::start().await;
    room.create_branch("B").await;
    room.create_branch("D").await;
    let customer = room.customer(CX1).await;

    // Publish on B, but then clear current_publication_id so B has no CURRENT publication
    let pub_time_b = Utc.with_ymd_and_hms(2026, 10, 13, 8, 0, 0).unwrap();
    room.publish_rate(
        "B",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.22000000")),
        Some(dec_str("0.23000000")),
        "ALL",
        pub_time_b,
    )
    .await;
    room.exec("UPDATE branches SET current_publication_id = NULL WHERE id = 'B'")
        .await;

    // Publish current JPY ALL line on branch D (positive control)
    let pub_time_d = Utc.with_ymd_and_hms(2026, 10, 13, 8, 30, 0).unwrap();
    room.publish_rate(
        "D",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.23000000")),
        Some(dec_str("0.24000000")),
        "ALL",
        pub_time_d,
    )
    .await;

    // Alert 1 on B, Alert 2 on D
    let id_b = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;
    let id_d = room
        .insert_alert(
            CX1,
            "D",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 2, "expected 2 alerts");

    let alert_d = alerts
        .iter()
        .find(|a| a["id"] == id_d.to_string())
        .expect("alert on D in response");
    assert_eq!(
        alert_d["currentRate"], "0.24000000",
        "row 4 positive control: alert on D must show currentRate"
    );
    assert_published_at_eq(
        &alert_d["ratePublishedAt"],
        pub_time_d,
        "row 4 positive control: alert on D must show ratePublishedAt",
    );
    assert_eq!(alert_d["branch"], "D", "row 4: branch D");
    assert_eq!(alert_d["unitSize"], "100.0000", "row 4: unitSize D");

    let alert_b = alerts
        .iter()
        .find(|a| a["id"] == id_b.to_string())
        .expect("alert on B in response");
    assert_eq!(alert_b["branch"], "B", "row 4: branch must match B");
    assert_eq!(
        alert_b["unitSize"], "100.0000",
        "row 4: unitSize must match 100.0000"
    );
    assert!(
        alert_b["currentRate"].is_null(),
        "row 4: B currentRate must be null when B has no current publication"
    );
    assert!(
        alert_b["ratePublishedAt"].is_null(),
        "row 4: B ratePublishedAt must be null when B has no current publication"
    );

    room.finish().await;
}

/// 5. The ALL line's unit_size ≠ the alert's → both null.
///    Positive control: a second alert on B with unit_size 1 matching the line,
///    showing 0.24000000 and the publication time.
#[actix_web::test]
async fn row5_unit_size_mismatch_both_null() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;
    let pub_time = Utc.with_ymd_and_hms(2026, 10, 13, 8, 30, 0).unwrap();
    // Publication line has unit_size = 1
    room.publish_rate(
        "B",
        "JPY",
        dec_str("1"),
        Some(dec_str("0.23000000")),
        Some(dec_str("0.24000000")),
        "ALL",
        pub_time,
    )
    .await;

    // Alert 1 on B with unit_size 100 (mismatched)
    let id_mismatch = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;
    // Alert 2 on B with unit_size 1 (matching positive control)
    let id_matching = room
        .insert_alert(CX1, "B", "JPY", "SELL", dec_str("1"), dec_str("0.24000000"))
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 2, "expected 2 alerts");

    let alert_match = alerts
        .iter()
        .find(|a| a["id"] == id_matching.to_string())
        .expect("matching alert in response");
    assert_eq!(
        alert_match["currentRate"], "0.24000000",
        "row 5 positive control: matching alert must show currentRate 0.24000000"
    );
    assert_published_at_eq(
        &alert_match["ratePublishedAt"],
        pub_time,
        "row 5 positive control: matching alert must show ratePublishedAt",
    );
    assert_eq!(alert_match["branch"], "B", "row 5: branch B");
    assert_eq!(alert_match["unitSize"], "1.0000", "row 5: unitSize 1.0000");

    let alert_mis = alerts
        .iter()
        .find(|a| a["id"] == id_mismatch.to_string())
        .expect("mismatched alert in response");
    assert_eq!(alert_mis["branch"], "B", "row 5: branch must match B");
    assert_eq!(
        alert_mis["unitSize"], "100.0000",
        "row 5: unitSize must match 100.0000"
    );
    assert!(
        alert_mis["currentRate"].is_null(),
        "row 5: currentRate must be null when ALL line unit_size differs"
    );
    assert!(
        alert_mis["ratePublishedAt"].is_null(),
        "row 5: ratePublishedAt must be null when ALL line unit_size differs"
    );

    room.finish().await;
}

/// 6. Only a non-ALL denomination line exists for the currency → both null.
///    Positive control: branch D with a JPY ALL line, and an alert on D.
#[actix_web::test]
async fn row6_non_all_denomination_line_both_null() {
    let room = Room::start().await;
    room.create_branch("B").await;
    room.create_branch("D").await;
    let customer = room.customer(CX1).await;
    let pub_time_b = Utc.with_ymd_and_hms(2026, 10, 13, 8, 30, 0).unwrap();
    // B has only a non-ALL denomination group (e.g. '10000')
    room.publish_rate(
        "B",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.23000000")),
        Some(dec_str("0.24000000")),
        "10000",
        pub_time_b,
    )
    .await;

    // D has a JPY ALL line (positive control)
    let pub_time_d = Utc.with_ymd_and_hms(2026, 10, 13, 8, 35, 0).unwrap();
    room.publish_rate(
        "D",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.23000000")),
        Some(dec_str("0.24000000")),
        "ALL",
        pub_time_d,
    )
    .await;

    let id_b = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;
    let id_d = room
        .insert_alert(
            CX1,
            "D",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 2, "expected 2 alerts");

    let alert_d = alerts
        .iter()
        .find(|a| a["id"] == id_d.to_string())
        .expect("alert on D in response");
    assert_eq!(
        alert_d["currentRate"], "0.24000000",
        "row 6 positive control: alert on D must show currentRate"
    );
    assert_published_at_eq(
        &alert_d["ratePublishedAt"],
        pub_time_d,
        "row 6 positive control: alert on D must show ratePublishedAt",
    );
    assert_eq!(alert_d["branch"], "D", "row 6: branch D");
    assert_eq!(alert_d["unitSize"], "100.0000", "row 6: unitSize D");

    let alert_b = alerts
        .iter()
        .find(|a| a["id"] == id_b.to_string())
        .expect("alert on B in response");
    assert_eq!(alert_b["branch"], "B", "row 6: branch must match B");
    assert_eq!(
        alert_b["unitSize"], "100.0000",
        "row 6: unitSize must match 100.0000"
    );
    assert!(
        alert_b["currentRate"].is_null(),
        "row 6: currentRate must be null when only non-ALL line exists on B"
    );
    assert!(
        alert_b["ratePublishedAt"].is_null(),
        "row 6: ratePublishedAt must be null when only non-ALL line exists on B"
    );

    room.finish().await;
}

/// 7. Another branch C has a newer publication with a JPY line, and B has none
///    → both null, because C's rate is never used.
///    Positive control: an alert on C showing C's rate and published_at.
#[actix_web::test]
async fn row7_other_branch_has_publication_b_has_none_both_null() {
    let room = Room::start().await;
    room.create_branch("B").await;
    room.create_branch("C").await;
    let customer = room.customer(CX1).await;
    let pub_time_c = Utc.with_ymd_and_hms(2026, 10, 13, 9, 0, 0).unwrap();
    // Branch C has a publication, but branch B has none
    room.publish_rate(
        "C",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.23000000")),
        Some(dec_str("0.24000000")),
        "ALL",
        pub_time_c,
    )
    .await;

    let id_b = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;
    let id_c = room
        .insert_alert(
            CX1,
            "C",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.24000000"),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 2, "expected 2 alerts");

    let alert_c = alerts
        .iter()
        .find(|a| a["id"] == id_c.to_string())
        .expect("alert on C in response");
    assert_eq!(
        alert_c["currentRate"], "0.24000000",
        "row 7 positive control: alert on C must show C's currentRate"
    );
    assert_published_at_eq(
        &alert_c["ratePublishedAt"],
        pub_time_c,
        "row 7 positive control: alert on C must show C's ratePublishedAt",
    );
    assert_eq!(alert_c["branch"], "C", "row 7: branch C");
    assert_eq!(alert_c["unitSize"], "100.0000", "row 7: unitSize C");

    let alert_b = alerts
        .iter()
        .find(|a| a["id"] == id_b.to_string())
        .expect("alert on B in response");
    assert_eq!(alert_b["branch"], "B", "row 7: branch must match B");
    assert_eq!(
        alert_b["unitSize"], "100.0000",
        "row 7: unitSize must match 100.0000"
    );
    assert!(
        alert_b["currentRate"].is_null(),
        "row 7: currentRate must be null because other branch C's publication is never used"
    );
    assert!(
        alert_b["ratePublishedAt"].is_null(),
        "row 7: ratePublishedAt must be null because other branch C's publication is never used"
    );

    room.finish().await;
}

/// 8. A second, later publication of B becomes current → currentRate and ratePublishedAt follow the current one, not the first.
#[actix_web::test]
async fn row8_second_later_publication_becomes_current() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;

    let time1 = Utc.with_ymd_and_hms(2026, 10, 13, 8, 0, 0).unwrap();
    let time2 = Utc.with_ymd_and_hms(2026, 10, 13, 9, 0, 0).unwrap();

    // First publication
    room.publish_rate(
        "B",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.22000000")),
        Some(dec_str("0.23000000")),
        "ALL",
        time1,
    )
    .await;

    // Second publication becomes current
    room.publish_rate(
        "B",
        "JPY",
        dec_str("100"),
        Some(dec_str("0.24000000")),
        Some(dec_str("0.25000000")),
        "ALL",
        time2,
    )
    .await;

    let alert_id = room
        .insert_alert(
            CX1,
            "B",
            "JPY",
            "SELL",
            dec_str("100"),
            dec_str("0.25000000"),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    assert_eq!(alerts.len(), 1, "expected 1 alert");
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(
        item["currentRate"], "0.25000000",
        "row 8: currentRate must follow the second (current) publication"
    );
    assert_published_at_eq(
        &item["ratePublishedAt"],
        time2,
        "row 8: ratePublishedAt must follow the second (current) publication",
    );
    assert_eq!(item["branch"], "B", "row 8: branch must be B");
    assert_eq!(
        item["unitSize"], "100.0000",
        "row 8: unitSize must be 100.0000"
    );

    room.finish().await;
}

// ------------------------------------------------------------ Ticket #3396 (C11) behaviour table tests

/// C11 row 1: alert with expires_at in the past → expired: true, expiresAt set
#[actix_web::test]
async fn c11_row1_alert_expired_in_past() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;
    let past = Utc::now() - chrono::Duration::hours(2);

    let alert_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "BUY",
            dec_str("1"),
            dec_str("35.00000000"),
            "UNKNOWN",
            Some(past),
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(
        item["expired"], true,
        "expired must be true for alert past expires_at"
    );
    assert_published_at_eq(&item["expiresAt"], past, "expiresAt must match past time");

    room.finish().await;
}

/// C11 row 2: alert with expires_at in the future, or none → expired: false; expiresAt the time or null
#[actix_web::test]
async fn c11_row2_alert_expires_in_future_or_none() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;
    let future = Utc::now() + chrono::Duration::hours(24);

    let alert_future_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "BUY",
            dec_str("1"),
            dec_str("35.00000000"),
            "UNKNOWN",
            Some(future),
        )
        .await;

    let alert_none_id = room
        .insert_alert_full(
            CX1,
            "B",
            "EUR",
            "BUY",
            dec_str("1"),
            dec_str("38.00000000"),
            "UNKNOWN",
            None,
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item_future = alerts
        .iter()
        .find(|a| a["id"] == alert_future_id.to_string())
        .expect("alert with future expiry in response");
    assert_eq!(
        item_future["expired"], false,
        "expired must be false for future alert"
    );
    assert_published_at_eq(
        &item_future["expiresAt"],
        future,
        "expiresAt must match future time",
    );

    let item_none = alerts
        .iter()
        .find(|a| a["id"] == alert_none_id.to_string())
        .expect("alert without expiry in response");
    assert_eq!(
        item_none["expired"], false,
        "expired must be false for alert with no expiry"
    );
    assert!(
        item_none
            .as_object()
            .expect("alert object")
            .contains_key("expiresAt"),
        "expiresAt must be sent, as null, when no expiry"
    );
    assert!(
        item_none["expiresAt"].is_null(),
        "expiresAt must be null when no expiry"
    );

    room.finish().await;
}

/// C11 row 3: relation stored SATISFIED → relation: "SATISFIED"
#[actix_web::test]
async fn c11_row3_relation_stored_satisfied() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;

    let alert_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "SELL",
            dec_str("1"),
            dec_str("36.00000000"),
            "SATISFIED",
            None,
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(item["relation"], "SATISFIED", "relation must be SATISFIED");

    room.finish().await;
}

/// C11 row 4: current line's snapshot fetched_at older than max_reference_age_hours → rateStale: true
#[actix_web::test]
async fn c11_row4_current_snapshot_stale() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;

    let now = Utc::now();
    let old_fetched_at = now - chrono::Duration::hours(5);
    let stale_snapshot = json!({
        "buy_mode": "AUTO",
        "sell_mode": "AUTO",
        "fetched_at": old_fetched_at.to_rfc3339(),
        "max_reference_age_hours": 2
    });

    room.publish_rate_with_snapshot(
        "B",
        "USD",
        dec_str("1"),
        Some(dec_str("35.00000000")),
        Some(dec_str("36.00000000")),
        "ALL",
        now,
        stale_snapshot,
    )
    .await;

    let alert_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "BUY",
            dec_str("1"),
            dec_str("35.00000000"),
            "UNKNOWN",
            None,
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(
        item["rateStale"], true,
        "rateStale must be true when snapshot fetched_at is too old"
    );

    room.finish().await;
}

/// C11 row 5: fresh snapshot → rateStale: false
#[actix_web::test]
async fn c11_row5_fresh_snapshot() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;

    let now = Utc::now();
    let fresh_snapshot = json!({
        "buy_mode": "AUTO",
        "sell_mode": "AUTO",
        "fetched_at": now.to_rfc3339(),
        "max_reference_age_hours": 24
    });

    room.publish_rate_with_snapshot(
        "B",
        "USD",
        dec_str("1"),
        Some(dec_str("35.00000000")),
        Some(dec_str("36.00000000")),
        "ALL",
        now,
        fresh_snapshot,
    )
    .await;

    let alert_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "BUY",
            dec_str("1"),
            dec_str("35.00000000"),
            "UNKNOWN",
            None,
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert_eq!(
        item["rateStale"], false,
        "rateStale must be false for fresh snapshot"
    );

    room.finish().await;
}

/// C11 row 6: no current publication (row 4's case) → currentRate: null, rateStale: null
#[actix_web::test]
async fn c11_row6_no_current_publication() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;

    let alert_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "BUY",
            dec_str("1"),
            dec_str("35.00000000"),
            "UNKNOWN",
            None,
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert!(
        item["currentRate"].is_null(),
        "currentRate must be null when no publication exists"
    );
    assert!(
        item.as_object()
            .expect("alert object")
            .contains_key("rateStale"),
        "rateStale must be sent, as null, when currentRate is null"
    );
    assert!(
        item["rateStale"].is_null(),
        "rateStale must be null when currentRate is null"
    );

    room.finish().await;
}

/// C11 row 7: two events, episodes 1 and 2 → lastEvent is episode 2's rate and time
#[actix_web::test]
async fn c11_row7_two_events_episodes_1_and_2() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;

    let now = Utc::now();
    let time1 = now - chrono::Duration::hours(1);
    let time2 = now;

    let (_, line_id1) = room
        .publish_rate_with_snapshot(
            "B",
            "USD",
            dec_str("1"),
            Some(dec_str("34.10000000")),
            Some(dec_str("35.10000000")),
            "ALL",
            time1,
            json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }),
        )
        .await;

    let (_, line_id2) = room
        .publish_rate_with_snapshot(
            "B",
            "USD",
            dec_str("1"),
            Some(dec_str("34.80000000")),
            Some(dec_str("35.80000000")),
            "ALL",
            time2,
            json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }),
        )
        .await;

    let alert_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "BUY",
            dec_str("1"),
            dec_str("35.00000000"),
            "UNKNOWN",
            None,
        )
        .await;

    room.insert_event(alert_id, line_id1, dec_str("34.10000000"), time1, 1)
        .await;
    room.insert_event(alert_id, line_id2, dec_str("34.80000000"), time2, 2)
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert!(!item["lastEvent"].is_null(), "lastEvent must not be null");
    assert_eq!(
        item["lastEvent"]["episodeNo"], 2,
        "lastEvent episodeNo must be 2"
    );
    assert_eq!(
        item["lastEvent"]["observedRate"], "34.80000000",
        "lastEvent observedRate must match episode 2"
    );
    assert_published_at_eq(
        &item["lastEvent"]["triggerAt"],
        time2,
        "lastEvent triggerAt must match episode 2",
    );

    room.finish().await;
}

/// C11 row 8: no event → lastEvent: null
#[actix_web::test]
async fn c11_row8_no_event() {
    let room = Room::start().await;
    room.create_branch("B").await;
    let customer = room.customer(CX1).await;

    let alert_id = room
        .insert_alert_full(
            CX1,
            "B",
            "USD",
            "BUY",
            dec_str("1"),
            dec_str("35.00000000"),
            "UNKNOWN",
            None,
        )
        .await;

    let alerts = my_alerts(&room, &customer).await;
    let item = alerts
        .iter()
        .find(|a| a["id"] == alert_id.to_string())
        .expect("alert in response");

    assert!(
        item.as_object()
            .expect("alert object")
            .contains_key("lastEvent"),
        "lastEvent must be sent, as null, when no event exists"
    );
    assert!(
        item["lastEvent"].is_null(),
        "lastEvent must be null when no event exists"
    );

    room.finish().await;
}
