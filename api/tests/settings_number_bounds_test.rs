//! #4971 (P3): the settings commands bound every number field to what its column and its screen can hold. A value
//! past the bound is 400 `validationFailed` naming that field; the bound itself is accepted.
//!
//! Written before the implementation by the ticket author's lane (claude/be38637e, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Rows (#4971 §1 bounds, §3 N1–N6). Each §1 row has one test: its max accepted (N1), then max+1, or min-1 where the
//! max is the column limit, refused on the field (N2).
//! - `rateAlertSettingsSave`: `defaultCooldownSeconds` 0..2,592,000; `maxActivePerCustomer` 1..100;
//!   `defaultLifetimeDays` 1..365.
//! - `bookingPolicySet`: `cancelCutoffMinutes` / `noShowGraceMinutes` 0..129,600; `confirmExpiryMinutes` 1..129,600.
//! - `slotRuleSet`: `capacity` 1..2,147,483,647.
//! - `slotCapacitySet`: `capacity` 1..2,147,483,647 (N3: 0 refused on `capacity`).
//! - `campaignCreate` and `campaignVersionSave`: `priority` 0..2,147,483,647.
//! - `couponIssue`: `count` 1..1,000 (N4: 0 refused on `count`, not an empty success).
//! - `ratePolicySave`: `guards` `min_pct` / `max_pct` each a signed `numeric(9,4)`, refused on `guards.min_pct` /
//!   `guards.max_pct` (N5: min 2, max 1 refused on `guards.min_pct`; N6: `1.23456` refused);
//!   `maxReferenceAgeHours` 1..168.
//! - `retentionSet`: `periodDays` 1..3,650.
//!
//! Today every max+1 / min-1 here is accepted (or, for the `i32` capacities, refused by the JSON reader with no
//! field), slot capacity 0 is accepted, coupon count 0 is an empty success, and a bad guard is stored and skipped.
//!
//! Every call is made by one organisation-wide EXECUTIVE, who holds each command's permission (`BranchesManage`,
//! `MarketingCampaigns`, `RatePolicySave`, `RateAlertSettingsSave`, `ConsentPolicyManage`). Needs
//! `TEST_DATABASE_URL`; nothing skips. Each test works in a schema of its own.

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
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
/// The branch the migrations seed; campaigns name it.
const MAIN: &str = "MAIN";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// The column limit of every `integer` field here.
const I32_MAX: i64 = i32::MAX as i64;

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
    /// The organisation-wide EXECUTIVE's header spec.
    exec: String,
    exec_id: String,
}

impl Room {
    async fn start() -> Self {
        common::install_test_pii_keys();
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
        let schema = unique("setbounds");
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
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, booking_enabled) \
             VALUES ($1, 'B01', $2::uuid, true)",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert a booking branch");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let exec_id = seed_person(
            &mut pool.acquire().await.unwrap(),
            ORG,
            &unique("EXECUTIVE"),
            &[RoleRow {
                role_code: Role::Executive.as_str(),
                branch_id: None,
                valid_from: from,
                valid_to: None,
            }],
        )
        .await
        .expect("insert the executive")
        .to_string();
        Self {
            pool,
            schema,
            exec: format!("{exec_id}|{}|*", Role::Executive.as_str()),
            exec_id,
        }
    }

    async fn exec_sql(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// An OPEN slot at B01 starting in an hour, nothing reserved.
    async fn slot(&self) -> Uuid {
        let id = Uuid::new_v4();
        let start = Utc::now() + chrono::Duration::hours(1);
        let date = (start + chrono::Duration::hours(7)).date_naive();
        sqlx::query(
            "INSERT INTO booking_slots \
               (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, 5, 0, 'OPEN')",
        )
        .bind(id)
        .bind(ORG)
        .bind(B01)
        .bind(date)
        .bind(start)
        .bind(start + chrono::Duration::minutes(30))
        .execute(&self.pool)
        .await
        .expect("seed a slot");
        id
    }

    /// An APPROVED campaign version running now at MAIN with one coupon rule: (campaign, rule).
    async fn coupon_campaign(&self) -> (Uuid, Uuid) {
        let campaign = Uuid::new_v4();
        let version = Uuid::new_v4();
        let rule = Uuid::new_v4();
        let by = &self.exec_id;
        self.exec_sql(&format!(
            "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
             VALUES ('{campaign}', '{ORG}', 'Bounds', NULL, '{by}', now())"
        ))
        .await;
        self.exec_sql(&format!(
            "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
               budget_thb, stack_group, priority, version_hash, created_by, created_at, edited_at) \
             VALUES ('{version}', '{ORG}', '{campaign}', 1, 'APPROVED', now() - interval '1 hour', \
               now() + interval '10 days', 10000.00, 'RATE', 1, 'v1_hash', '{by}', now(), now())"
        ))
        .await;
        self.exec_sql(&format!(
            "UPDATE campaigns SET current_version_id = '{version}' WHERE id = '{campaign}'"
        ))
        .await;
        self.exec_sql(&format!(
            "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
             VALUES ('{version}', 0, 0)"
        ))
        .await;
        self.exec_sql(&format!(
            "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ('{version}', '{MAIN}')"
        ))
        .await;
        self.exec_sql(&format!(
            "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
               calculation, caps, requires_coupon, rule_no) \
             VALUES ('{rule}', '{version}', 'RATE_IMPROVEMENT', 'PRICE', '{{}}', '{{}}', '{{}}', true, 1)"
        ))
        .await;
        (campaign, rule)
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ session

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
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

async fn call(room: &Room, method: Method, uri: &str, body: Value) -> Answer {
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
        .insert_header((AS, room.exec.clone()))
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

/// N1: the bound itself is accepted.
fn accepted(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: expected 200: {}", a.text);
}

/// N2–N6: 400 `validationFailed` on `field`.
fn refused_on(a: &Answer, field: &str, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "{what}: expected 400: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["code"], "validationFailed",
        "{what}: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"], field,
        "{what}: the refused field: {}",
        a.text
    );
}

// ------------------------------------------------------------ one request per command

async fn alert_settings(room: &Room, cooldown: i64, max_active: i64, lifetime: i64) -> Answer {
    call(
        room,
        Method::PUT,
        "/api/rate-alert-settings",
        json!({
            "defaultCooldownSeconds": cooldown,
            "maxActivePerCustomer": max_active,
            "defaultLifetimeDays": lifetime,
            "observePolicy": "ALL",
        }),
    )
    .await
}

async fn booking_policy(room: &Room, cutoff: i64, grace: i64, expiry: i64) -> Answer {
    call(
        room,
        Method::PUT,
        "/api/booking-policy",
        json!({
            "cancelCutoffMinutes": cutoff,
            "noShowGraceMinutes": grace,
            "confirmExpiryMinutes": expiry,
        }),
    )
    .await
}

async fn slot_rule(room: &Room, capacity: i64) -> Answer {
    call(
        room,
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        json!({ "branch": B01, "slotMinutes": 30, "capacity": capacity, "horizonDays": 14 }),
    )
    .await
}

async fn slot_capacity(room: &Room, slot: Uuid, capacity: i64) -> Answer {
    call(
        room,
        Method::PUT,
        &format!("/api/booking-slots/{slot}/capacity"),
        json!({ "branch": B01, "capacity": capacity }),
    )
    .await
}

fn campaign_terms(priority: i64) -> Value {
    json!({
        "startsAt": "2027-01-01T00:00:00Z",
        "endsAt": "2027-02-01T00:00:00Z",
        "budgetThb": "10000.00",
        "stackGroup": "RATE",
        "priority": priority,
        "branches": [MAIN],
    })
}

async fn campaign_create(room: &Room, priority: i64) -> Answer {
    let mut body = campaign_terms(priority);
    body["name"] = json!(unique("Bounds"));
    call(room, Method::POST, "/api/campaigns", body).await
}

async fn campaign_draft(room: &Room, campaign: &str, priority: i64) -> Answer {
    call(
        room,
        Method::PUT,
        &format!("/api/campaigns/{campaign}/draft"),
        campaign_terms(priority),
    )
    .await
}

async fn coupon_issue(room: &Room, campaign: Uuid, rule: Uuid, count: i64) -> Answer {
    let expires = (Utc::now() + chrono::Duration::days(5)).to_rfc3339();
    call(
        room,
        Method::POST,
        &format!("/api/campaigns/{campaign}/coupons"),
        json!({ "ruleId": rule.to_string(), "count": count, "expiresAt": expires }),
    )
    .await
}

/// A rate policy for B01 with USD `buy` guards `min_pct`/`max_pct` and `maxReferenceAgeHours`.
async fn rate_policy(room: &Room, min_pct: &str, max_pct: &str, max_age_hours: i64) -> Answer {
    call(
        room,
        Method::POST,
        "/api/rate-policies",
        json!({
            "branch": B01,
            "buyAdjustPct": "0",
            "sellAdjustPct": "0",
            "guards": {
                "USD": {
                    "buy": { "min_pct": min_pct, "max_pct": max_pct },
                    "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
                }
            },
            "maxReferenceAgeHours": max_age_hours,
            "onStale": "LIMIT",
            "staleLimitThb": "5000.00",
            "reason": "#4971 bounds",
        }),
    )
    .await
}

async fn retention(room: &Room, period_days: i64) -> Answer {
    call(
        room,
        Method::PUT,
        "/api/retention-policies/IDENTITY_VALUE",
        json!({ "dataType": "IDENTITY_VALUE", "periodDays": period_days }),
    )
    .await
}

// ------------------------------------------------------------ §1 rows: max accepted (N1), past it refused (N2)

#[actix_web::test]
async fn alert_default_cooldown_accepts_30_days_and_refuses_one_second_more() {
    let room = Room::start().await;
    accepted(
        &alert_settings(&room, 2_592_000, 10, 30).await,
        "defaultCooldownSeconds 2,592,000",
    );
    refused_on(
        &alert_settings(&room, 2_592_001, 10, 30).await,
        "defaultCooldownSeconds",
        "defaultCooldownSeconds 2,592,001",
    );
    room.finish().await;
}

#[actix_web::test]
async fn alert_max_active_per_customer_accepts_100_and_refuses_101() {
    let room = Room::start().await;
    accepted(
        &alert_settings(&room, 3600, 100, 30).await,
        "maxActivePerCustomer 100",
    );
    refused_on(
        &alert_settings(&room, 3600, 101, 30).await,
        "maxActivePerCustomer",
        "maxActivePerCustomer 101",
    );
    room.finish().await;
}

#[actix_web::test]
async fn alert_default_lifetime_accepts_365_days_and_refuses_366() {
    let room = Room::start().await;
    accepted(
        &alert_settings(&room, 3600, 10, 365).await,
        "defaultLifetimeDays 365",
    );
    refused_on(
        &alert_settings(&room, 3600, 10, 366).await,
        "defaultLifetimeDays",
        "defaultLifetimeDays 366",
    );
    room.finish().await;
}

#[actix_web::test]
async fn booking_cancel_cutoff_and_no_show_grace_accept_129600_and_refuse_129601() {
    let room = Room::start().await;
    accepted(
        &booking_policy(&room, 129_600, 129_600, 30).await,
        "cancelCutoffMinutes and noShowGraceMinutes 129,600",
    );
    refused_on(
        &booking_policy(&room, 129_601, 20, 30).await,
        "cancelCutoffMinutes",
        "cancelCutoffMinutes 129,601",
    );
    refused_on(
        &booking_policy(&room, 45, 129_601, 30).await,
        "noShowGraceMinutes",
        "noShowGraceMinutes 129,601",
    );
    room.finish().await;
}

#[actix_web::test]
async fn booking_confirm_expiry_accepts_129600_and_refuses_129601() {
    let room = Room::start().await;
    accepted(
        &booking_policy(&room, 45, 20, 129_600).await,
        "confirmExpiryMinutes 129,600",
    );
    refused_on(
        &booking_policy(&room, 45, 20, 129_601).await,
        "confirmExpiryMinutes",
        "confirmExpiryMinutes 129,601",
    );
    room.finish().await;
}

#[actix_web::test]
async fn slot_rule_capacity_accepts_the_column_limit_and_refuses_one_more() {
    let room = Room::start().await;
    accepted(
        &slot_rule(&room, I32_MAX).await,
        "slotRuleSet capacity 2,147,483,647",
    );
    refused_on(
        &slot_rule(&room, I32_MAX + 1).await,
        "capacity",
        "slotRuleSet capacity 2,147,483,648",
    );
    room.finish().await;
}

#[actix_web::test]
async fn slot_capacity_accepts_the_column_limit_and_refuses_one_more() {
    let room = Room::start().await;
    let slot = room.slot().await;
    accepted(
        &slot_capacity(&room, slot, I32_MAX).await,
        "slotCapacitySet capacity 2,147,483,647",
    );
    refused_on(
        &slot_capacity(&room, slot, I32_MAX + 1).await,
        "capacity",
        "slotCapacitySet capacity 2,147,483,648",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_create_priority_accepts_the_column_limit_and_refuses_minus_1() {
    let room = Room::start().await;
    accepted(
        &campaign_create(&room, I32_MAX).await,
        "campaignCreate priority 2,147,483,647",
    );
    refused_on(
        &campaign_create(&room, -1).await,
        "priority",
        "campaignCreate priority -1",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_draft_save_priority_accepts_the_column_limit_and_refuses_minus_1() {
    let room = Room::start().await;
    let created = campaign_create(&room, 0).await;
    accepted(&created, "campaignCreate priority 0");
    let campaign = created.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("campaignCreate answers the campaign's id: {}", created.text))
        .to_owned();
    accepted(
        &campaign_draft(&room, &campaign, I32_MAX).await,
        "campaignVersionSave priority 2,147,483,647",
    );
    refused_on(
        &campaign_draft(&room, &campaign, -1).await,
        "priority",
        "campaignVersionSave priority -1",
    );
    room.finish().await;
}

#[actix_web::test]
async fn coupon_count_accepts_1000_and_refuses_1001() {
    let room = Room::start().await;
    let (campaign, rule) = room.coupon_campaign().await;
    let a = coupon_issue(&room, campaign, rule, 1000).await;
    accepted(&a, "couponIssue count 1,000");
    assert_eq!(
        a.body["coupons"].as_array().map(Vec::len),
        Some(1000),
        "1,000 coupons issued: {}",
        a.text
    );
    refused_on(
        &coupon_issue(&room, campaign, rule, 1001).await,
        "count",
        "couponIssue count 1,001",
    );
    room.finish().await;
}

#[actix_web::test]
async fn rate_policy_guards_accept_the_widest_numeric_9_4_and_refuse_past_it() {
    let room = Room::start().await;
    accepted(
        &rate_policy(&room, "-99999.9999", "99999.9999", 24).await,
        "guards min -99,999.9999 max 99,999.9999",
    );
    refused_on(
        &rate_policy(&room, "-1.0000", "100000.0000", 24).await,
        "guards.max_pct",
        "guards max_pct 100,000",
    );
    refused_on(
        &rate_policy(&room, "-100000.0000", "1.0000", 24).await,
        "guards.min_pct",
        "guards min_pct -100,000",
    );
    room.finish().await;
}

#[actix_web::test]
async fn rate_policy_max_reference_age_accepts_168_hours_and_refuses_169() {
    let room = Room::start().await;
    accepted(
        &rate_policy(&room, "-1.0000", "1.0000", 168).await,
        "maxReferenceAgeHours 168",
    );
    refused_on(
        &rate_policy(&room, "-1.0000", "1.0000", 169).await,
        "maxReferenceAgeHours",
        "maxReferenceAgeHours 169",
    );
    room.finish().await;
}

#[actix_web::test]
async fn retention_period_accepts_3650_days_and_refuses_3651() {
    let room = Room::start().await;
    accepted(&retention(&room, 3650).await, "periodDays 3,650");
    refused_on(
        &retention(&room, 3651).await,
        "periodDays",
        "periodDays 3,651",
    );
    room.finish().await;
}

// ------------------------------------------------------------ N3–N6

#[actix_web::test]
async fn n3_slot_capacity_0_is_refused_on_capacity() {
    let room = Room::start().await;
    let slot = room.slot().await;
    refused_on(
        &slot_capacity(&room, slot, 0).await,
        "capacity",
        "slotCapacitySet capacity 0",
    );
    room.finish().await;
}

#[actix_web::test]
async fn n4_coupon_count_0_is_refused_on_count_not_an_empty_success() {
    let room = Room::start().await;
    let (campaign, rule) = room.coupon_campaign().await;
    refused_on(
        &coupon_issue(&room, campaign, rule, 0).await,
        "count",
        "couponIssue count 0",
    );
    room.finish().await;
}

#[actix_web::test]
async fn n5_guards_with_min_above_max_are_refused_on_guards_min_pct() {
    let room = Room::start().await;
    refused_on(
        &rate_policy(&room, "2", "1", 24).await,
        "guards.min_pct",
        "guards min 2 max 1",
    );
    room.finish().await;
}

#[actix_web::test]
async fn n6_a_guard_with_a_fifth_decimal_is_refused() {
    let room = Room::start().await;
    refused_on(
        &rate_policy(&room, "1.23456", "2.0000", 24).await,
        "guards.min_pct",
        "guards min_pct 1.23456",
    );
    room.finish().await;
}
