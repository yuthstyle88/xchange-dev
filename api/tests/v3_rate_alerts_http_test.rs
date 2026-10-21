//! Ticket #1429 (RA-2b): the HTTP half of rate alerts, over RA-2's `RateAlertService` (design
//! `docs/design/blueprint/rate-alerts.md` §3, §4.3, §4.4, §9.1 RA-2b).
//!
//! Written before the implementation by the test-writer claude/5d45b017, on the BE lane controller claude/ad05bf8d's
//! order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **What is pinned** (#1429 §7–§9, with the lane controller's rulings Q1 and Q2 on #1429):
//! - `POST /api/app/rate-alerts` (`rateAlertCreate`) `{branch, currency, customerSide, condition, target, expiresAt?}`
//!   and `POST /api/app/rate-alerts/{id}/pause|resume|delete` `{expectedVersion}`, for `CustomerSelf`;
//!   `PUT /api/rate-alert-settings` (`rateAlertSettingsSave`) for `RateAlertSettingsSave`, EXECUTIVE only;
//!   the views `my_rate_alerts` (`SignedIn`, reading only the caller's own alerts by `customer_id`, an empty list for an
//!   actor with no customer, like `my_notifications`; lane controller's ruling on #1429) and `rate_alert_settings`
//!   (`RateAlertSettingsSave`).
//! - Every command takes `Idempotency-Key`: the same key and body replays the first answer and writes once; the same
//!   key with another body is `idempotencyConflict`.
//! - **Q1:** another customer's alert answers exactly as an unknown id, `alertNotFound` (RA-2 ruling 2: "never
//!   reveal"), not `outOfScope`.
//! - **Q2:** a refusal's wire `code` is its reason when the service names one (`alertSettingsMissing`,
//!   `alertLimitReached`, `alertNotFound`), and `validationFailed` only when it names none.
//! - Without `RateAlertSettingsSave`, the settings command and view answer `roleNotAllowedHere` (ADMIN included).
//!
//! **Wire shapes this file fixes** ([required], the DTOs are new): the create, pause, resume and delete answers carry
//! the alert's `id` (string) and `version` (number); `my_rate_alerts`'s `data.alerts` is a list of
//! `{id, version, currency, customerSide, condition, target, enabled}` (`target` a decimal string; the side in the
//! customer's words, `CUSTOMER_BUY` / `CUSTOMER_SELL`); `rate_alert_settings`'s `data` is `null` before the first save,
//! then `{defaultCooldownSeconds, maxActivePerCustomer, defaultLifetimeDays, observePolicy}`, the same keys the
//! settings command takes and answers. Any further field is the implementer's.
//!
//! **Red on `main` by result:** no route and no view exist, so every answer is a 404 or has no code. The callers are
//! Blueprint `access::Actor`s (a customer carries `customer_id`), handed to the seams through
//! `SessionReader::callers` with empty legacy sessions; this needs no change from #654 (lane controller, #1429).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test` (never the queue's port 6381); nothing
//! skips. Each test works in a schema of its own.

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
use rust_decimal::Decimal;
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
const SETTINGS: &str = "/api/rate-alert-settings";
const MAIN: &str = "MAIN";
/// The seeded organisation (`20261011100000_organizations.sql`).
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const CX1: &str = "CX-RA2B-1";
const CX2: &str = "CX-RA2B-2";
const NOT_ALLOWED: &str = "roleNotAllowedHere";

/// The Blueprint staff roles, used only to make actors.
const STAFF_ROLES: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::VerificationReviewer,
];

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn dec(v: &Value) -> Decimal {
    Decimal::from_str(
        v.as_str()
            .unwrap_or_else(|| panic!("a decimal string: {v}")),
    )
    .unwrap_or_else(|e| panic!("a decimal string {v}: {e}"))
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
        let schema = unique("ra2b");
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

    /// A signed-in staff member with `role`, organisation-wide.
    async fn staff(&self, role: Role) -> Who {
        let id = self.user().await;
        Who(format!("{id}|{}|", role.as_str()))
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

/// `pause`, `resume` or `delete` of alert `id` with `expectedVersion`.
async fn lifecycle(
    room: &Room,
    who: &Who,
    id: &str,
    step: &str,
    version: i64,
    key: &str,
) -> Answer {
    call(
        room,
        who,
        Method::POST,
        &format!("{CREATE}/{id}/{step}"),
        Some((json!({ "expectedVersion": version }), key)),
    )
    .await
}

async fn my_alerts(room: &Room, who: &Who) -> Vec<Value> {
    let a = call(room, who, Method::GET, "/api/v3/views/my_rate_alerts", None).await;
    let body = a.ok("my_rate_alerts");
    body["data"]["alerts"]
        .as_array()
        .unwrap_or_else(|| panic!("my_rate_alerts answers data.alerts: {}", a.text))
        .clone()
}

/// The `id` and `version` of an alert answer.
fn id_version(a: &Answer, what: &str) -> (String, i64) {
    let body = a.ok(what);
    let id = body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: the alert's id: {}", a.text))
        .to_owned();
    let version = body["version"]
        .as_i64()
        .unwrap_or_else(|| panic!("{what}: the alert's version: {}", a.text));
    (id, version)
}

fn settings_body(max: i64) -> Value {
    json!({
        "defaultCooldownSeconds": 3600,
        "maxActivePerCustomer": max,
        "defaultLifetimeDays": 30,
        "observePolicy": "ALL_FRESH",
    })
}

// ------------------------------------------------------------ create, and the view reads it back

#[actix_web::test]
async fn create_answers_the_alert_and_my_rate_alerts_reads_it_back() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let a = create(&room, &cx1, usd_above("34.10"), &unique("key")).await;
    let (id, version) = id_version(&a, "create");
    assert_eq!(version, 1, "a new alert is version 1: {}", a.text);
    // R1: the customer's side becomes the shop side once (CUSTOMER_BUY → the shop sells)
    let stored: (String, String) =
        sqlx::query_as("SELECT customer_id, shop_side FROM rate_alerts WHERE id::text = $1")
            .bind(&id)
            .fetch_one(&room.pool)
            .await
            .expect("the stored alert");
    assert_eq!(stored, (CX1.to_owned(), "SELL".to_owned()));

    let rows = my_alerts(&room, &cx1).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row["id"], json!(id));
    assert_eq!(row["version"], json!(1));
    assert_eq!(row["currency"], json!("USD"));
    assert_eq!(
        row["customerSide"],
        json!("CUSTOMER_BUY"),
        "the side in the customer's words"
    );
    assert_eq!(row["condition"], json!("ABOVE"));
    assert_eq!(dec(&row["target"]), Decimal::from_str("34.10").unwrap());
    assert_eq!(row["enabled"], json!(true));
    room.finish().await;
}

#[actix_web::test]
async fn create_replays_the_same_key_and_refuses_the_same_key_with_another_body() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let key = unique("key");
    let first = create(&room, &cx1, usd_above("34.10"), &key).await;
    let first = first.ok("the first create");
    let again = create(&room, &cx1, usd_above("34.10"), &key).await;
    assert_eq!(
        again.ok("the replay"),
        first,
        "the replay is the first answer"
    );
    assert_eq!(room.alerts().await, 1, "one alert after the replay");
    let other = create(&room, &cx1, usd_above("35.00"), &key).await;
    assert_eq!(other.code(), "idempotencyConflict", "{}", other.text);
    assert_eq!(room.alerts().await, 1, "the conflict wrote nothing");
    room.finish().await;
}

// ------------------------------------------------------------ create's refusals (Q2: the reason is the code)

#[actix_web::test]
async fn create_without_a_settings_row_is_alert_settings_missing() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;
    let a = create(&room, &cx1, usd_above("34.10"), &unique("key")).await;
    assert_eq!(a.code(), "alertSettingsMissing", "{}", a.text);
    assert_eq!(room.alerts().await, 0);
    room.finish().await;
}

#[actix_web::test]
async fn create_at_the_limit_is_alert_limit_reached() {
    let room = Room::start().await;
    room.settings(1).await;
    let cx1 = room.customer(CX1).await;
    create(&room, &cx1, usd_above("34.10"), &unique("key"))
        .await
        .ok("the first alert");
    let a = create(&room, &cx1, usd_above("35.00"), &unique("key")).await;
    assert_eq!(a.code(), "alertLimitReached", "{}", a.text);
    assert_eq!(room.alerts().await, 1);
    room.finish().await;
}

#[actix_web::test]
async fn a_bad_target_is_validation_failed() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    for target in ["0", "-1", "34.123456789"] {
        let a = create(&room, &cx1, usd_above(target), &unique("key")).await;
        assert_eq!(a.code(), "validationFailed", "target {target}: {}", a.text);
    }
    assert_eq!(room.alerts().await, 0);
    room.finish().await;
}

// ------------------------------------------------------------ pause, resume, delete

#[actix_web::test]
async fn pause_resume_and_delete_work_through_their_routes() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let (id, v1) = id_version(
        &create(&room, &cx1, usd_above("34.10"), &unique("key")).await,
        "create",
    );

    let (_, v2) = id_version(
        &lifecycle(&room, &cx1, &id, "pause", v1, &unique("key")).await,
        "pause",
    );
    assert!(v2 > v1, "pause moves the version");
    let rows = my_alerts(&room, &cx1).await;
    assert_eq!(rows[0]["enabled"], json!(false), "paused: {rows:?}");
    assert_eq!(rows[0]["version"], json!(v2));

    let (_, v3) = id_version(
        &lifecycle(&room, &cx1, &id, "resume", v2, &unique("key")).await,
        "resume",
    );
    assert!(v3 > v2, "resume moves the version");
    let rows = my_alerts(&room, &cx1).await;
    assert_eq!(rows[0]["enabled"], json!(true), "resumed: {rows:?}");

    lifecycle(&room, &cx1, &id, "delete", v3, &unique("key"))
        .await
        .ok("delete");
    let deleted: i64 = room
        .count(&format!(
            "SELECT count(*) FROM rate_alerts WHERE id::text = '{id}' AND deleted_at IS NOT NULL"
        ))
        .await;
    assert_eq!(deleted, 1, "delete sets deleted_at");
    room.finish().await;
}

#[actix_web::test]
async fn a_lifecycle_key_replays_once_and_another_body_is_a_conflict() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let (id, v1) = id_version(
        &create(&room, &cx1, usd_above("34.10"), &unique("key")).await,
        "create",
    );
    let key = unique("key");
    let first = lifecycle(&room, &cx1, &id, "pause", v1, &key).await;
    let first = first.ok("the first pause");
    let again = lifecycle(&room, &cx1, &id, "pause", v1, &key).await;
    assert_eq!(
        again.ok("the replay"),
        first,
        "the replay is the first answer"
    );
    let version: i32 = sqlx::query_scalar("SELECT version FROM rate_alerts WHERE id::text = $1")
        .bind(&id)
        .fetch_one(&room.pool)
        .await
        .expect("the alert's version");
    assert_eq!(
        i64::from(version),
        first["version"].as_i64().unwrap(),
        "paused once"
    );
    let other = lifecycle(&room, &cx1, &id, "pause", v1 + 7, &key).await;
    assert_eq!(other.code(), "idempotencyConflict", "{}", other.text);
    room.finish().await;
}

#[actix_web::test]
async fn a_stale_expected_version_is_stale_version() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let (id, v1) = id_version(
        &create(&room, &cx1, usd_above("34.10"), &unique("key")).await,
        "create",
    );
    for step in ["pause", "resume", "delete"] {
        let a = lifecycle(&room, &cx1, &id, step, v1 + 5, &unique("key")).await;
        assert_eq!(a.code(), "staleVersion", "{step}: {}", a.text);
    }
    room.finish().await;
}

// ------------------------------------------------------------ another customer's alert (Q1)

#[actix_web::test]
async fn another_customers_alert_is_alert_not_found_and_stays_unchanged() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let cx2 = room.customer(CX2).await;
    let (id, v1) = id_version(
        &create(&room, &cx1, usd_above("34.10"), &unique("key")).await,
        "create",
    );
    for step in ["pause", "resume", "delete"] {
        let a = lifecycle(&room, &cx2, &id, step, v1, &unique("key")).await;
        assert_eq!(
            a.code(),
            "alertNotFound",
            "{step} by another customer: {}",
            a.text
        );
    }
    let unknown = lifecycle(
        &room,
        &cx2,
        "00000000-0000-0000-0000-00000000dead",
        "pause",
        1,
        &unique("key"),
    )
    .await;
    assert_eq!(
        unknown.code(),
        "alertNotFound",
        "an unknown id answers the same: {}",
        unknown.text
    );
    let rows = my_alerts(&room, &cx1).await;
    assert_eq!(rows[0]["version"], json!(v1), "unchanged: {rows:?}");
    assert_eq!(rows[0]["enabled"], json!(true));
    room.finish().await;
}

#[actix_web::test]
async fn my_rate_alerts_never_lists_another_customers_alert() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let cx2 = room.customer(CX2).await;
    let (mine, _) = id_version(
        &create(&room, &cx2, usd_above("34.10"), &unique("key")).await,
        "cx2's create",
    );
    create(&room, &cx1, usd_above("35.00"), &unique("key"))
        .await
        .ok("cx1's create");
    let rows = my_alerts(&room, &cx2).await;
    assert_eq!(
        rows.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(),
        vec![json!(mine)],
        "only cx2's own alert"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_caller_with_no_customer_reads_an_empty_my_rate_alerts() {
    // #1429 §7 [required]: the view reads only the caller's own alerts by `customer_id`, and "an actor with none
    // gets an empty list" — never every customer's alerts (dell-claude-2's review of these tests, 02:35Z).
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;
    let cx2 = room.customer(CX2).await;
    create(&room, &cx1, usd_above("34.10"), &unique("key"))
        .await
        .ok("cx1's create");
    create(&room, &cx2, usd_above("35.00"), &unique("key"))
        .await
        .ok("cx2's create");
    for role in STAFF_ROLES.iter().copied() {
        let who = room.staff(role).await;
        assert_eq!(
            my_alerts(&room, &who).await,
            Vec::<Value>::new(),
            "{} has no customer_id: an empty list",
            role.as_str()
        );
    }
    room.finish().await;
}

// ------------------------------------------------------------ settings

#[actix_web::test]
async fn the_settings_view_is_null_before_the_first_save_then_reads_it_back() {
    let room = Room::start().await;
    let exec = room.staff(Role::Executive).await;
    let a = call(
        &room,
        &exec,
        Method::GET,
        "/api/v3/views/rate_alert_settings",
        None,
    )
    .await;
    let body = a.ok("the settings view");
    assert!(
        body.as_object().is_some_and(|o| o.contains_key("data")),
        "a view answer with `data`: {}",
        a.text
    );
    assert_eq!(
        body["data"],
        Value::Null,
        "null before the first save: {}",
        a.text
    );

    let saved = call(
        &room,
        &exec,
        Method::PUT,
        SETTINGS,
        Some((settings_body(5), &unique("key"))),
    )
    .await;
    let saved = saved.ok("the save");
    for (k, v) in settings_body(5).as_object().unwrap() {
        assert_eq!(&saved[k], v, "the save answers {k}");
    }
    let a = call(
        &room,
        &exec,
        Method::GET,
        "/api/v3/views/rate_alert_settings",
        None,
    )
    .await;
    let data = a.ok("the settings view")["data"].clone();
    for (k, v) in settings_body(5).as_object().unwrap() {
        assert_eq!(&data[k], v, "the view reads back {k}: {}", a.text);
    }
    room.finish().await;
}

#[actix_web::test]
async fn the_settings_save_replays_its_key_and_refuses_another_body() {
    let room = Room::start().await;
    let exec = room.staff(Role::Executive).await;
    let key = unique("key");
    let first = call(
        &room,
        &exec,
        Method::PUT,
        SETTINGS,
        Some((settings_body(5), &key)),
    )
    .await;
    let first = first.ok("the first save");
    let again = call(
        &room,
        &exec,
        Method::PUT,
        SETTINGS,
        Some((settings_body(5), &key)),
    )
    .await;
    assert_eq!(again.ok("the replay"), first);
    let other = call(
        &room,
        &exec,
        Method::PUT,
        SETTINGS,
        Some((settings_body(6), &key)),
    )
    .await;
    assert_eq!(other.code(), "idempotencyConflict", "{}", other.text);
    let max: i32 = sqlx::query_scalar("SELECT max_active_per_customer FROM rate_alert_settings")
        .fetch_one(&room.pool)
        .await
        .expect("the one settings row");
    assert_eq!(max, 5, "the conflict changed nothing");
    room.finish().await;
}

#[actix_web::test]
async fn a_save_with_settings_makes_create_possible() {
    let room = Room::start().await;
    let exec = room.staff(Role::Executive).await;
    let cx1 = room.customer(CX1).await;
    let before = create(&room, &cx1, usd_above("34.10"), &unique("key")).await;
    assert_eq!(before.code(), "alertSettingsMissing", "{}", before.text);
    call(
        &room,
        &exec,
        Method::PUT,
        SETTINGS,
        Some((settings_body(5), &unique("key"))),
    )
    .await
    .ok("the save");
    create(&room, &cx1, usd_above("34.10"), &unique("key"))
        .await
        .ok("create after the save");
    room.finish().await;
}

/// §11.2: `RateAlertSettingsSave` (`RATES_STOCK` `FULL`) is held by `EXECUTIVE` and
/// `RATE_MANAGER`; every role without it in `RoleGrants::seed(role)` (ADMIN included) is refused.
#[actix_web::test]
async fn only_holders_of_rate_alert_settings_save_may_save_or_read_the_settings() {
    use xc_auth::domain::access::{Permission, RoleGrants};

    let room = Room::start().await;
    assert!(
        STAFF_ROLES
            .iter()
            .any(|r| !RoleGrants::seed(*r).contains(&Permission::RateAlertSettingsSave)),
        "§11.2 must leave some role without RateAlertSettingsSave, or this test refuses nobody"
    );
    for role in STAFF_ROLES
        .iter()
        .copied()
        .filter(|r| !RoleGrants::seed(*r).contains(&Permission::RateAlertSettingsSave))
    {
        let who = room.staff(role).await;
        let a = call(
            &room,
            &who,
            Method::PUT,
            SETTINGS,
            Some((settings_body(5), &unique("key"))),
        )
        .await;
        assert_eq!(a.code(), NOT_ALLOWED, "{} saves: {}", role.as_str(), a.text);
        let a = call(
            &room,
            &who,
            Method::GET,
            "/api/v3/views/rate_alert_settings",
            None,
        )
        .await;
        assert_eq!(a.code(), NOT_ALLOWED, "{} reads: {}", role.as_str(), a.text);
    }
    let cx1 = room.customer(CX1).await;
    let a = call(
        &room,
        &cx1,
        Method::PUT,
        SETTINGS,
        Some((settings_body(5), &unique("key"))),
    )
    .await;
    assert_eq!(a.code(), NOT_ALLOWED, "a customer saves: {}", a.text);
    assert_eq!(
        room.count("SELECT count(*) FROM rate_alert_settings").await,
        0,
        "no refused save wrote a row"
    );
    room.finish().await;
}

// ------------------------------------------------------------ expectedVersion validation (#1470)

#[actix_web::test]
async fn lifecycle_commands_refuse_out_of_range_expected_version_with_validation_failed() {
    let room = Room::start().await;
    room.settings(10).await;
    let cx1 = room.customer(CX1).await;

    const OVERFLOW: i64 = 2_147_483_648; // i32::MAX as i64 + 1
    const NEGATIVE: i64 = -1;

    for bad_version in [OVERFLOW, NEGATIVE] {
        // 1. pause on an active alert
        let (id_pause, v_pause) = id_version(
            &create(&room, &cx1, usd_above("34.10"), &unique("key")).await,
            "create for pause",
        );
        let a = lifecycle(&room, &cx1, &id_pause, "pause", bad_version, &unique("key")).await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "pause with version {bad_version}: expected 400, got {} {}",
            a.status,
            a.text
        );
        assert_eq!(
            a.code(),
            "validationFailed",
            "pause with version {bad_version}: expected validationFailed, got {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["field"].as_str(),
            Some("expectedVersion"),
            "pause with version {bad_version}: expected field 'expectedVersion', got {}",
            a.text
        );
        let v_after: i32 =
            sqlx::query_scalar("SELECT version FROM rate_alerts WHERE id::text = $1")
                .bind(&id_pause)
                .fetch_one(&room.pool)
                .await
                .expect("read version after refused pause");
        let enabled_after: bool =
            sqlx::query_scalar("SELECT enabled FROM rate_alerts WHERE id::text = $1")
                .bind(&id_pause)
                .fetch_one(&room.pool)
                .await
                .expect("read enabled after refused pause");
        assert_eq!(
            i64::from(v_after),
            v_pause,
            "pause refusal wrote no version"
        );
        assert!(enabled_after, "pause refusal left alert enabled");

        // 2. resume on a paused alert
        let (id_resume, v_created) = id_version(
            &create(&room, &cx1, usd_above("34.10"), &unique("key")).await,
            "create for resume",
        );
        let (_, v_resume) = id_version(
            &lifecycle(&room, &cx1, &id_resume, "pause", v_created, &unique("key")).await,
            "pause to set up resume",
        );
        let a = lifecycle(
            &room,
            &cx1,
            &id_resume,
            "resume",
            bad_version,
            &unique("key"),
        )
        .await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "resume with version {bad_version}: expected 400, got {} {}",
            a.status,
            a.text
        );
        assert_eq!(
            a.code(),
            "validationFailed",
            "resume with version {bad_version}: expected validationFailed, got {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["field"].as_str(),
            Some("expectedVersion"),
            "resume with version {bad_version}: expected field 'expectedVersion', got {}",
            a.text
        );
        let v_after: i32 =
            sqlx::query_scalar("SELECT version FROM rate_alerts WHERE id::text = $1")
                .bind(&id_resume)
                .fetch_one(&room.pool)
                .await
                .expect("read version after refused resume");
        let enabled_after: bool =
            sqlx::query_scalar("SELECT enabled FROM rate_alerts WHERE id::text = $1")
                .bind(&id_resume)
                .fetch_one(&room.pool)
                .await
                .expect("read enabled after refused resume");
        assert_eq!(
            i64::from(v_after),
            v_resume,
            "resume refusal wrote no version"
        );
        assert!(!enabled_after, "resume refusal left alert paused");

        // 3. delete on an active alert
        let (id_delete, v_delete) = id_version(
            &create(&room, &cx1, usd_above("34.10"), &unique("key")).await,
            "create for delete",
        );
        let a = lifecycle(
            &room,
            &cx1,
            &id_delete,
            "delete",
            bad_version,
            &unique("key"),
        )
        .await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "delete with version {bad_version}: expected 400, got {} {}",
            a.status,
            a.text
        );
        assert_eq!(
            a.code(),
            "validationFailed",
            "delete with version {bad_version}: expected validationFailed, got {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["field"].as_str(),
            Some("expectedVersion"),
            "delete with version {bad_version}: expected field 'expectedVersion', got {}",
            a.text
        );
        let deleted_count: i64 = room
            .count(&format!(
                "SELECT count(*) FROM rate_alerts WHERE id::text = '{id_delete}' AND deleted_at IS NOT NULL"
            ))
            .await;
        assert_eq!(deleted_count, 0, "delete refusal did not set deleted_at");
        let v_after: i32 =
            sqlx::query_scalar("SELECT version FROM rate_alerts WHERE id::text = $1")
                .bind(&id_delete)
                .fetch_one(&room.pool)
                .await
                .expect("read version after refused delete");
        assert_eq!(
            i64::from(v_after),
            v_delete,
            "delete refusal wrote no version"
        );
    }

    room.finish().await;
}
