//! R2a (feat/be-gaps-b; design `docs/design/blueprint/rate-alerts.md` R2, R2a, §9 row RA-4c, §9.1 row RA-4c): when a
//! publication's line has a `unit_size` other than the alert's, the evaluator disables the alert (`UNIT_CHANGED`) and,
//! in the same transaction, tells the customer once: one `notifications` row, template `rate_alert_unit_changed`,
//! `event_id = "rate_alert_unit_changed:<alert id>"`, object `{rate_alert, <alert id>}`, params only `currency` and
//! `side` (customer words), no `rate_alert_events` row.
//!
//! Written before the implementation by the sub-controller (claude/2ef7d5a6). Do not edit, delete or weaken these
//! tests; add your own in another file. The harness below is copied from `rate_alert_evaluator_test.rs` (RA-4).
//!
//! [required] by these tests:
//! - `xc_notify::templates::rate_alert_unit_changed::{KEY, Template}`: key `rate_alert_unit_changed`, version 1,
//!   `Transactional`; each of TH/EN/ZH renders a non-empty title and a body naming the `currency` param.
//! - The evaluator's `UNIT_CHANGED` branch calls `notify_in(&mut tx, …)` before `continue`, in the same transaction
//!   as the `UPDATE`: a failing `notify_in` fails the evaluation and leaves the alert enabled.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod common;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row};
use tokio::sync::Mutex;
use uuid::Uuid;
use xc_alert::application::evaluator::RateAlertEvaluator;
use xc_kernel::ids::BranchId;
use xc_kernel::notifications::{NotificationId, NotificationRequest, Notifications};
use xc_notify::application::notification_service::NotificationService;
use xc_notify::application::template_registry::TemplateRegistry;
use xc_notify::domain::template::{Locale, NotificationTemplate, TemplateKind};
use xc_notify::templates::rate_alert_unit_changed::{
    KEY, Template as RateAlertUnitChangedTemplate,
};

pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
pub const EXEC: &str = "c1000000-0000-0000-0000-000000000001";
pub const USER_CX1: &str = "c2000000-0000-0000-0000-000000000001";
pub const USER_CX2: &str = "c2000000-0000-0000-0000-000000000002";
pub const MAIN: &str = "MAIN";
pub const CX1: &str = "CX-1";
pub const CX2: &str = "CX-2";

pub const MUST_FINISH: Duration = Duration::from_secs(25);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn org() -> Uuid {
    Uuid::parse_str(ORG).expect("uuid")
}

fn exec() -> Uuid {
    Uuid::parse_str(EXEC).expect("uuid")
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("decimal")
}

fn fresh_auto() -> Value {
    json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" })
}

fn fresh_manual() -> Value {
    json!({ "buy_mode": "MANUAL", "sell_mode": "MANUAL", "reason": "manual override" })
}

fn stale_auto() -> Value {
    json!({
        "buy_mode": "AUTO",
        "sell_mode": "AUTO",
        "fetched_at": "2020-01-01T00:00:00Z",
        "max_reference_age_hours": 24
    })
}

pub struct Line {
    pub currency: &'static str,
    pub unit: &'static str,
    pub buy: Option<&'static str>,
    pub sell: Option<&'static str>,
    pub snapshot: Value,
}

pub fn line(
    currency: &'static str,
    unit: &'static str,
    buy: &'static str,
    sell: &'static str,
) -> Line {
    Line {
        currency,
        unit,
        buy: Some(buy),
        sell: Some(sell),
        snapshot: fresh_auto(),
    }
}

pub fn manual_line(
    currency: &'static str,
    unit: &'static str,
    buy: &'static str,
    sell: &'static str,
) -> Line {
    Line {
        currency,
        unit,
        buy: Some(buy),
        sell: Some(sell),
        snapshot: fresh_manual(),
    }
}

pub fn stale_line(
    currency: &'static str,
    unit: &'static str,
    buy: &'static str,
    sell: &'static str,
) -> Line {
    Line {
        currency,
        unit,
        buy: Some(buy),
        sell: Some(sell),
        snapshot: stale_auto(),
    }
}

/// Fake notifier that records EVERY `notify_in` call with NO in-memory deduplication.
/// This ensures duplicate notifications are observable and fail tests (kills M1).
#[derive(Clone, Default)]
pub struct FakeNotifications {
    requests: Arc<Mutex<Vec<NotificationRequest>>>,
    should_fail: Arc<AtomicBool>,
}

impl FakeNotifications {
    pub fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            should_fail: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn captured(&self) -> Vec<NotificationRequest> {
        self.requests.lock().await.clone()
    }

    pub fn fail_next(&self) {
        self.should_fail.store(true, Ordering::SeqCst);
    }
}

impl Notifications for FakeNotifications {
    fn notify_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, Postgres>,
        request: &'a NotificationRequest,
    ) -> BoxFuture<'a, AppResult<NotificationId>> {
        Box::pin(async move {
            if self.should_fail.load(Ordering::SeqCst) {
                return Err(AppError::Internal(
                    "simulated notification failure for rollback test".into(),
                ));
            }
            // NO deduplication: record every call so duplicate notifications are visible
            let mut reqs = self.requests.lock().await;
            reqs.push(request.clone());
            Ok(NotificationId(Uuid::new_v4().to_string()))
        })
    }
}

pub struct AlertBuilder {
    pub id: Uuid,
    pub org_id: Uuid,
    pub customer_id: String,
    pub branch_id: String,
    pub currency: String,
    pub shop_side: String,
    pub unit_size: Decimal,
    pub condition: String,
    pub target: Decimal,
    pub enabled: bool,
    pub cooldown_seconds: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub relation: String,
    pub last_publication_version: Option<i32>,
    pub episode_no: i32,
    pub disabled_reason: Option<String>,
}

impl Default for AlertBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertBuilder {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            org_id: org(),
            customer_id: CX1.to_string(),
            branch_id: MAIN.to_string(),
            currency: "USD".to_string(),
            shop_side: "BUY".to_string(),
            unit_size: Decimal::ONE,
            condition: "ABOVE".to_string(),
            target: dec("35.00"),
            enabled: true,
            cooldown_seconds: 0,
            expires_at: None,
            relation: "UNKNOWN".to_string(),
            last_publication_version: Some(1),
            episode_no: 0,
            disabled_reason: None,
        }
    }

    pub fn with_target(mut self, target: &str) -> Self {
        self.target = dec(target);
        self
    }

    pub fn with_currency(mut self, curr: &str) -> Self {
        self.currency = curr.to_string();
        self
    }

    pub fn with_unit_size(mut self, unit: &str) -> Self {
        self.unit_size = dec(unit);
        self
    }

    pub fn with_relation(mut self, rel: &str) -> Self {
        self.relation = rel.to_string();
        self
    }

    pub fn with_cooldown(mut self, cooldown: i32) -> Self {
        self.cooldown_seconds = cooldown;
        self
    }

    pub fn with_last_version(mut self, v: Option<i32>) -> Self {
        self.last_publication_version = v;
        self
    }

    pub fn with_episode(mut self, ep: i32) -> Self {
        self.episode_no = ep;
        self
    }

    pub fn with_expires_at(mut self, exp: Option<DateTime<Utc>>) -> Self {
        self.expires_at = exp;
        self
    }
}

pub struct AlertRow {
    pub id: Uuid,
    pub enabled: bool,
    pub relation: String,
    pub last_publication_version: Option<i32>,
    pub episode_no: i32,
    pub disabled_reason: Option<String>,
}

pub struct EventRow {
    pub id: Uuid,
    pub alert_id: Uuid,
    pub published_rate_line_id: Uuid,
    pub observed_rate: Decimal,
    pub episode_no: i32,
    pub trigger_at: DateTime<Utc>,
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

impl TestDb {
    pub async fn world() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The rate_alert_evaluator tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`."
        );

        let schema = unique("r2a");
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
            .max_connections(15)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply migrations");

        let db = Self { pool, schema };

        db.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .await;
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ('{EXEC}', '{ORG}', 'Exec', 'ACTIVE', 'test'), \
             ('{USER_CX1}', '{ORG}', 'Somchai', 'ACTIVE', 'test'), \
             ('{USER_CX2}', '{ORG}', 'Somsri', 'ACTIVE', 'test')"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, user_id, created_at) VALUES \
             ('{CX1}', '{ORG}', '{CX1}', 'Somchai', 'LIVE', '{USER_CX1}', now()), \
             ('{CX2}', '{ORG}', '{CX2}', 'Somsri', 'LIVE', '{USER_CX2}', now())"
        ))
        .await;

        db.settings(86400, 10, Some(30), "ALL_FRESH").await;

        // Clean any leftover notifications
        db.exec(
            "TRUNCATE notification_deliveries, notifications, rate_alert_events, rate_alerts CASCADE",
        )
        .await;

        db
    }

    pub async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    pub async fn settings(&self, cooldown: i32, max: i32, lifetime: Option<i32>, policy: &str) {
        sqlx::query(
            "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, max_active_per_customer, \
             default_lifetime_days, observe_policy, updated_by, updated_at) VALUES ($1, $2, $3, $4, $5, $6, now()) \
             ON CONFLICT (organization_id) DO UPDATE SET default_cooldown_seconds = EXCLUDED.default_cooldown_seconds, \
             max_active_per_customer = EXCLUDED.max_active_per_customer, \
             default_lifetime_days = EXCLUDED.default_lifetime_days, observe_policy = EXCLUDED.observe_policy",
        )
        .bind(org())
        .bind(cooldown)
        .bind(max)
        .bind(lifetime)
        .bind(policy)
        .bind(exec())
        .execute(&self.pool)
        .await
        .expect("write settings");
    }

    pub async fn publish(&self, branch: &str, version: i32, lines: Vec<Line>) -> Uuid {
        let approval: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2, '00000000-0000-0000-0000-000000000001') \
             RETURNING id",
        )
        .bind(branch)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("approved request");

        let publication: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, $2, now(), $3, 'test') RETURNING id",
        )
        .bind(branch)
        .bind(version)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("publication");

        for l in lines {
            sqlx::query(
                "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
                 source_snapshot) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6)",
            )
            .bind(publication)
            .bind(l.currency)
            .bind(dec(l.unit))
            .bind(l.buy.map(dec))
            .bind(l.sell.map(dec))
            .bind(&l.snapshot)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("line {} of v{version}: {e}", l.currency));
        }

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("update current publication");

        publication
    }

    pub async fn publish_with_groups(
        &self,
        branch: &str,
        version: i32,
        lines: Vec<(&'static str, Line)>,
    ) -> (Uuid, Vec<Uuid>) {
        let approval: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2, '00000000-0000-0000-0000-000000000001') \
             RETURNING id",
        )
        .bind(branch)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("approved request");

        let publication: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, $2, now(), $3, 'test') RETURNING id",
        )
        .bind(branch)
        .bind(version)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("publication");

        let mut ids = Vec::new();
        for (group, l) in lines {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, \
                 source_snapshot) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7) RETURNING id",
            )
            .bind(publication)
            .bind(l.currency)
            .bind(group)
            .bind(dec(l.unit))
            .bind(l.buy.map(dec))
            .bind(l.sell.map(dec))
            .bind(&l.snapshot)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("line {} ({group}) of v{version}: {e}", l.currency));
            ids.push(id);
        }

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("update current publication");

        (publication, ids)
    }

    pub async fn insert_alert(&self, a: &AlertBuilder) -> Uuid {
        let key = unique("alert_key");
        sqlx::query(
            "INSERT INTO rate_alerts (id, organization_id, customer_id, branch_id, currency_code, \
             shop_side, unit_size, condition, target, enabled, cooldown_seconds, expires_at, \
             relation, last_publication_version, episode_no, disabled_reason, version, idempotency_key, \
             created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, 1, $17, now(), now())",
        )
        .bind(a.id)
        .bind(a.org_id)
        .bind(&a.customer_id)
        .bind(&a.branch_id)
        .bind(&a.currency)
        .bind(&a.shop_side)
        .bind(a.unit_size)
        .bind(&a.condition)
        .bind(a.target)
        .bind(a.enabled)
        .bind(a.cooldown_seconds)
        .bind(a.expires_at)
        .bind(&a.relation)
        .bind(a.last_publication_version)
        .bind(a.episode_no)
        .bind(&a.disabled_reason)
        .bind(key)
        .execute(&self.pool)
        .await
        .expect("insert alert");

        a.id
    }

    pub async fn get_alert(&self, id: Uuid) -> AlertRow {
        let row = sqlx::query(
            "SELECT id, enabled, relation, last_publication_version, episode_no, disabled_reason \
             FROM rate_alerts WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("fetch alert");

        AlertRow {
            id: row.get("id"),
            enabled: row.get("enabled"),
            relation: row.get("relation"),
            last_publication_version: row.get("last_publication_version"),
            episode_no: row.get("episode_no"),
            disabled_reason: row.get("disabled_reason"),
        }
    }

    pub async fn count_events(&self, alert_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM rate_alert_events WHERE alert_id = $1")
            .bind(alert_id)
            .fetch_one(&self.pool)
            .await
            .expect("count events")
    }

    pub async fn get_events(&self, alert_id: Uuid) -> Vec<EventRow> {
        let rows = sqlx::query(
            "SELECT id, alert_id, published_rate_line_id, observed_rate, episode_no, trigger_at \
             FROM rate_alert_events WHERE alert_id = $1 ORDER BY episode_no ASC",
        )
        .bind(alert_id)
        .fetch_all(&self.pool)
        .await
        .expect("fetch events");

        rows.into_iter()
            .map(|r| EventRow {
                id: r.get("id"),
                alert_id: r.get("alert_id"),
                published_rate_line_id: r.get("published_rate_line_id"),
                observed_rate: r.get("observed_rate"),
                episode_no: r.get("episode_no"),
                trigger_at: r.get("trigger_at"),
            })
            .collect()
    }

    pub async fn count_notifications(&self, event_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&self.pool)
            .await
            .expect("count notifications")
    }

    pub async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect admin");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop schema");
        admin.close().await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// R2a
// ─────────────────────────────────────────────────────────────────────────────

fn real_notifications(pool: &PgPool) -> Arc<dyn Notifications> {
    Arc::new(NotificationService::new(
        pool.clone(),
        Arc::new(TemplateRegistry::generated()),
    ))
}

#[derive(Debug)]
struct NoticeRow {
    recipient: Uuid,
    event_id: String,
    template_key: String,
    object_type: Option<String>,
    object_id: Option<String>,
    params: Value,
}

async fn unit_changed_notices(db: &TestDb) -> Vec<NoticeRow> {
    sqlx::query(
        "SELECT recipient_user_id, event_id, template_key, object_type, object_id, params \
         FROM notifications WHERE template_key = 'rate_alert_unit_changed' ORDER BY seq",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read notifications")
    .into_iter()
    .map(|r| NoticeRow {
        recipient: r.get("recipient_user_id"),
        event_id: r.get("event_id"),
        template_key: r.get("template_key"),
        object_type: r.get("object_type"),
        object_id: r.get("object_id"),
        params: r.get("params"),
    })
    .collect()
}

fn jpy_per_100() -> AlertBuilder {
    AlertBuilder::new()
        .with_currency("JPY")
        .with_unit_size("100")
        .with_target("24.00")
        .with_relation("NOT_SATISFIED")
        .with_last_version(Some(1))
}

#[tokio::test]
async fn r2a_1_a_changed_unit_disables_the_alert_and_writes_exactly_one_notice() {
    let db = TestDb::world().await;
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), real_notifications(&db.pool));
    let alert_id = db.insert_alert(&jpy_per_100()).await;

    let v2 = db
        .publish(MAIN, 2, vec![line("JPY", "1", "0.24", "0.25")])
        .await;
    evaluator
        .evaluate_publication(&BranchId::new(MAIN), v2)
        .await
        .expect("evaluate v2");

    let alert = db.get_alert(alert_id).await;
    assert!(!alert.enabled, "disabled");
    assert_eq!(alert.disabled_reason.as_deref(), Some("UNIT_CHANGED"));
    assert_eq!(
        db.count_events(alert_id).await,
        0,
        "not a crossing: no rate_alert_events row"
    );

    let notices = unit_changed_notices(&db).await;
    assert_eq!(notices.len(), 1, "told once: {notices:?}");
    let n = &notices[0];
    assert_eq!(
        n.recipient,
        Uuid::parse_str(USER_CX1).unwrap(),
        "the alert's customer"
    );
    assert_eq!(n.template_key, KEY);
    assert_eq!(n.event_id, format!("rate_alert_unit_changed:{alert_id}"));
    assert_eq!(n.object_type.as_deref(), Some("rate_alert"));
    assert_eq!(n.object_id.as_deref(), Some(alert_id.to_string().as_str()));
    assert_eq!(
        n.params,
        json!({ "currency": "JPY", "side": "CUSTOMER_SELL" }),
        "params only currency and side, in customer words (shop BUY = customer sells)"
    );

    db.finish().await;
}

#[tokio::test]
async fn r2a_2_a_redelivery_or_a_second_publication_writes_no_second_notice() {
    let db = TestDb::world().await;
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), real_notifications(&db.pool));
    let alert_id = db.insert_alert(&jpy_per_100()).await;
    let branch = BranchId::new(MAIN);

    let v2 = db
        .publish(MAIN, 2, vec![line("JPY", "1", "0.24", "0.25")])
        .await;
    evaluator
        .evaluate_publication(&branch, v2)
        .await
        .expect("v2");
    evaluator
        .evaluate_publication(&branch, v2)
        .await
        .expect("v2 again");
    let v3 = db
        .publish(MAIN, 3, vec![line("JPY", "1", "0.23", "0.24")])
        .await;
    evaluator
        .evaluate_publication(&branch, v3)
        .await
        .expect("v3");

    assert_eq!(unit_changed_notices(&db).await.len(), 1, "still told once");
    assert_eq!(db.count_events(alert_id).await, 0);

    db.finish().await;
}

#[tokio::test]
async fn r2a_3_the_same_unit_writes_no_unit_changed_notice() {
    let db = TestDb::world().await;
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), real_notifications(&db.pool));
    let alert_id = db.insert_alert(&jpy_per_100()).await;

    let v2 = db
        .publish(MAIN, 2, vec![line("JPY", "100", "23.00", "23.50")])
        .await;
    evaluator
        .evaluate_publication(&BranchId::new(MAIN), v2)
        .await
        .expect("evaluate v2");

    assert!(db.get_alert(alert_id).await.enabled, "still enabled");
    assert!(
        unit_changed_notices(&db).await.is_empty(),
        "no unit-changed notice"
    );

    db.finish().await;
}

#[tokio::test]
async fn r2a_4_a_failing_notice_rolls_the_disable_back_with_it() {
    let db = TestDb::world().await;
    let fake = Arc::new(FakeNotifications::new());
    fake.fail_next();
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake.clone());
    let alert_id = db.insert_alert(&jpy_per_100()).await;

    let v2 = db
        .publish(MAIN, 2, vec![line("JPY", "1", "0.24", "0.25")])
        .await;
    let answer = evaluator
        .evaluate_publication(&BranchId::new(MAIN), v2)
        .await;
    assert!(
        answer.is_err(),
        "the notice is in the evaluation's transaction: its failure fails it"
    );

    let alert = db.get_alert(alert_id).await;
    assert!(alert.enabled, "the UPDATE rolled back with the notice");
    assert_eq!(alert.disabled_reason, None);
    assert_eq!(alert.last_publication_version, Some(1), "v2 not recorded");

    db.finish().await;
}

#[tokio::test]
async fn r2a_5_the_request_names_the_template_event_and_object_of_r2a() {
    let db = TestDb::world().await;
    let fake = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake.clone());
    let alert_id = db.insert_alert(&jpy_per_100()).await;

    let v2 = db
        .publish(MAIN, 2, vec![line("JPY", "1", "0.24", "0.25")])
        .await;
    evaluator
        .evaluate_publication(&BranchId::new(MAIN), v2)
        .await
        .expect("evaluate v2");

    let sent = fake.captured().await;
    assert_eq!(sent.len(), 1, "one notify_in call");
    let req = &sent[0];
    assert_eq!(req.recipient_user_id(), USER_CX1);
    assert_eq!(req.template_key(), "rate_alert_unit_changed");
    assert_eq!(
        req.event_id(),
        format!("rate_alert_unit_changed:{alert_id}")
    );
    let object = req.object().expect("an object");
    assert_eq!(object.object_type, "rate_alert");
    assert_eq!(object.object_id, alert_id.to_string());
    let keys: Vec<&str> = req.params().keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["currency", "side"]);

    db.finish().await;
}

#[test]
fn r2a_6_the_template_is_transactional_version_1_and_names_the_currency_in_each_language() {
    let t = RateAlertUnitChangedTemplate;
    assert_eq!(t.key(), "rate_alert_unit_changed");
    assert_eq!(KEY, "rate_alert_unit_changed");
    assert_eq!(t.version(), 1);
    assert!(matches!(t.kind(), TemplateKind::Transactional));
    let mut params = BTreeMap::new();
    params.insert("currency".to_string(), "JPY".to_string());
    params.insert("side".to_string(), "CUSTOMER_SELL".to_string());
    let mut bodies = Vec::new();
    for locale in [Locale::Th, Locale::En, Locale::Zh] {
        let r = t.render(locale, &params);
        assert!(!r.title.trim().is_empty(), "a title");
        assert!(
            r.body.contains("JPY"),
            "the body names the currency: {}",
            r.body
        );
        bodies.push(r.body);
    }
    assert_ne!(bodies[0], bodies[1], "TH and EN differ");
    assert_ne!(bodies[1], bodies[2], "EN and ZH differ");
    assert!(
        TemplateRegistry::generated().get(KEY).is_some(),
        "the generated registry lists it"
    );
}
