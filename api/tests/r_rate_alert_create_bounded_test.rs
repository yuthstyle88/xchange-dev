//! R-rate-alert up-front tests (#2054 row 3, design `docs/design/blueprint/bounded-inputs.md` §4 row R-*) — written
//! before the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller
//! claude/56a74fcd. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `POST /api/app/rate-alerts` (`rateAlertCreate`, customer-facing): `target` is stored in
//! `rate_alerts.target numeric(20,8) NOT NULL CHECK (target > 0)` (`20261107100000_rate_alerts.sql:26`), so it is
//! bounded on both sides: 0 < target < 10^12, at most 8 decimals. Main parses it with `Decimal::from_str`
//! (`rate_alert_create.rs:88`); the service checks `> 0` and the scale (`rate_alert_service.rs:188`) but not the
//! magnitude, so a target of 10^12 or more reaches the INSERT and answers 500 with database text. Every refusal is 400
//! `validationFailed` **naming `target`**, and writes no alert.
//!
//! Harness copied from `v3_rate_alerts_http_test.rs`. Needs `TEST_DATABASE_URL` naming a database whose name ends in
//! `_test`; nothing skips.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const CREATE: &str = "/api/app/rate-alerts";
const MAIN: &str = "MAIN";
/// The seeded organisation (`20261011100000_organizations.sql`).
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const CX1: &str = "CX-RA2B-1";
const CX2: &str = "CX-RA2B-2";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    /// A schema with MAIN, two customers of the organisation and MAIN's publication v1 (USD 1, buy 33.50, sell 34.00).
    /// No `rate_alert_settings` row: each test writes the one it needs.
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
        let schema = unique("r_rate_alert");
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
        room.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .await;
        room.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) VALUES \
             ('{CX1}', '{ORG}', '{CX1}', 'Somchai', 'LIVE', now()), \
             ('{CX2}', '{ORG}', '{CX2}', 'Somsri', 'LIVE', now())"
        ))
        .await;
        room.publish_usd().await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// MAIN's current publication v1: USD per 1, buy 33.50, sell 34.00, a fresh AUTO side.
    async fn publish_usd(&self) {
        let approval: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2, '00000000-0000-0000-0000-000000000001') \
             RETURNING id",
        )
        .bind(MAIN)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("an approved request for the publication");
        let publication: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, 1, now(), $2, 'test') RETURNING id",
        )
        .bind(MAIN)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("the publication");
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
             source_snapshot) VALUES (gen_random_uuid(), $1, 'USD', 1, 33.50, 34.00, $2)",
        )
        .bind(publication)
        .bind(json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }))
        .execute(&self.pool)
        .await
        .expect("the USD line");
        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(MAIN)
            .execute(&self.pool)
            .await
            .expect("make it current");
    }

    /// The organisation's settings row, written directly (as RA-2's tests do), by a user of the organisation.
    async fn settings(&self, max: i32) {
        let by = self.user().await;
        sqlx::query(
            "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, max_active_per_customer, \
             default_lifetime_days, observe_policy, updated_by, updated_at) \
             VALUES ($1::uuid, 86400, $2, 30, 'ALL_FRESH', $3::uuid, now())",
        )
        .bind(ORG)
        .bind(max)
        .bind(&by)
        .execute(&self.pool)
        .await
        .expect("write the settings row");
    }

    /// A `users` row of the organisation; returns its id.
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

    /// A signed-in customer `customer_id` (an organisation-wide CUSTOMER row).
    async fn customer(&self, customer_id: &str) -> Who {
        let id = self.user().await;
        Who(format!("{id}|CUSTOMER|{customer_id}"))
    }

    async fn alerts(&self) -> i64 {
        self.count("SELECT count(*) FROM rate_alerts").await
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

// ------------------------------------------------------------ who is signed in

/// `<user id>|<ROLE>|<customer id or empty>`.
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
    fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }

    fn ok(&self, what: &str) -> Value {
        assert_eq!(self.status, StatusCode::OK, "{what}: {}", self.text);
        self.body.clone()
    }
}

async fn call(
    room: &Room,
    who: &Who,
    method: Method,
    uri: &str,
    body: Option<(Value, &str)>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
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
        .insert_header((AS, who.0.clone()));
    if let Some((body, key)) = body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn usd_above(target: &str) -> Value {
    json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "condition": "ABOVE",
        "target": target,
    })
}

async fn create(room: &Room, who: &Who, body: Value, key: &str) -> Answer {
    call(room, who, Method::POST, CREATE, Some((body, key))).await
}

// ------------------------------------------------------------ target: numeric(20,8), > 0

fn assert_target_refused(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(a.code(), "validationFailed", "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["field"],
        json!("target"),
        "{what}: {}",
        a.text
    );
}

async fn ready_room() -> (Room, Who) {
    let room = Room::start().await;
    room.settings(50).await;
    let cx1 = room.customer(CX1).await;
    (room, cx1)
}

#[actix_web::test]
async fn the_largest_target_the_column_holds_is_accepted_and_stored_exactly() {
    let (room, cx1) = ready_room().await;
    let a = create(
        &room,
        &cx1,
        usd_above("999999999999.99999999"),
        &unique("key"),
    )
    .await;
    a.ok("target 10^12 - 10^-8");
    let stored: String = sqlx::query_scalar("SELECT target::text FROM rate_alerts")
        .fetch_one(&room.pool)
        .await
        .expect("one alert");
    assert_eq!(stored, "999999999999.99999999");
    room.finish().await;
}

#[actix_web::test]
async fn a_target_of_ten_to_the_twelve_is_400_on_target_and_writes_nothing() {
    let (room, cx1) = ready_room().await;
    for target in ["1000000000000", "1000000000000.00000000", &"9".repeat(30)] {
        let a = create(&room, &cx1, usd_above(target), &unique("key")).await;
        assert_target_refused(&a, target);
    }
    assert_eq!(room.alerts().await, 0, "no alert written");
    room.finish().await;
}

#[actix_web::test]
async fn the_smallest_positive_target_is_accepted() {
    let (room, cx1) = ready_room().await;
    create(&room, &cx1, usd_above("0.00000001"), &unique("key"))
        .await
        .ok("target 10^-8");
    assert_eq!(room.alerts().await, 1);
    room.finish().await;
}

#[actix_web::test]
async fn zero_negative_and_a_ninth_decimal_are_refused_on_target() {
    let (room, cx1) = ready_room().await;
    for target in ["0", "0.00000000", "-1", "34.123456789", "0.000000001"] {
        let a = create(&room, &cx1, usd_above(target), &unique("key")).await;
        assert_target_refused(&a, target);
    }
    assert_eq!(room.alerts().await, 0, "no alert written");
    room.finish().await;
}

#[actix_web::test]
async fn extra_zero_decimals_are_the_same_target() {
    let (room, cx1) = ready_room().await;
    create(&room, &cx1, usd_above("34.100000000"), &unique("key"))
        .await
        .ok("a ninth decimal that is zero");
    let stored: String = sqlx::query_scalar("SELECT target::text FROM rate_alerts")
        .fetch_one(&room.pool)
        .await
        .expect("one alert");
    assert_eq!(stored, "34.10000000");
    room.finish().await;
}

#[actix_web::test]
async fn text_that_is_not_a_plain_decimal_is_refused_on_target() {
    let (room, cx1) = ready_room().await;
    for target in ["", "abc", "1e5", " 34.10", "34.10 ", "+34.10", ".5", "5."] {
        let a = create(&room, &cx1, usd_above(target), &unique("key")).await;
        assert_target_refused(&a, &format!("{target:?}"));
    }
    assert_eq!(room.alerts().await, 0, "no alert written");
    room.finish().await;
}
