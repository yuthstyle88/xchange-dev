//! Ticket #1009 (AL-4; design `docs/design/blueprint/rate-alerts.md` §5, §7–§9).
//!
//! Up-front specification tests for the rate alert evaluator, outbox handler and template,
//! written by test-writer `gemini/studio-14`, ordered by lane controller `claude/ad05bf8d`.
//!
//! Fix round per lane controller ruling (issuecomment-6119134353) on `claude/14cc24bd`'s review
//! (issuecomment-6119122796):
//! 1. Pin `last_publication_version` (R13): verified after each publication in row 1, and in row 2
//!    with a redelivery of stale v2 after v3 proving v2 is ignored and relation stays unchanged (kills M2).
//! 2. End-to-end outbox handler test: drives `RateAlertsHandler::deliver` via an `OutboxEvent` of kind
//!    `rate_alerts.publication`, asserting `handler(state)` is `Some` and kind is `RATE_ALERTS_PUBLICATION`,
//!    with idempotent redelivery.
//! 3. Version order (design §8 item 3): if v3 is delivered first, v2 is skipped because
//!    `last_publication_version` is already 3.
//! 4. Observable single notification: `FakeNotifications` records every `notify_in` call with NO in-memory
//!    deduplication, asserting exactly one notification per fire (kills M1).
//! 5. Template test: `RateAlertCrossedTemplate` verified for key `rate_alert_crossed`, `Transactional`,
//!    and non-empty TH/EN/ZH rendering. Params hold NO digit run (display values: `currency`, `side`, `relation`),
//!    and `NotificationObject` holds the reference `("rate_alert_event", event id)`.
//!
//! Covers all 10 edge cases of §9 (design §9.1 row RA-4):
//! 1. an `ABOVE` alert, cooldown 0, publications v1 below, v2 above, v3 above, v4 below, v5 above
//!    -> events for v2 and v5 only, episodes 1 and 2
//! 2. the v2 row delivered twice -> one event, one notification
//! 3. two evaluators on one publication at once -> one event (SKIP LOCKED + last_publication_version)
//! 4. paused at v2, resumed at v3 (above) -> no fire
//! 5. an expired alert -> no fire
//! 6. a line with unit_size != the alert's -> disabled UNIT_CHANGED, no event
//! 7. ALL_FRESH, a stale side -> skipped, relation unchanged
//! 8. ALL_FRESH, a fresh MANUAL side -> compared like a fresh AUTO side
//! 9. the notification's params -> no customer name, phone or transaction amount (no digit run; object link)
//! 10. a rollback inside the evaluation -> no event, no notification, alert unchanged

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
use xc_notify::domain::template::{Locale, NotificationTemplate, TemplateKind};
use xc_notify::templates::rate_alert_crossed::Template as RateAlertCrossedTemplate;
use xc_outbox::domain::event::{OutboxEvent, OutboxStatus};
use xc_outbox::domain::handler::{DeliveryResult, FinalOutcome};
use xc_rate::application::publication::alert_event::RATE_ALERTS_PUBLICATION;
use xchange::bootstrap::AppState;
use xchange::bootstrap::outbox_handlers;

pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
pub const EXEC: &str = "c1000000-0000-0000-0000-000000000001";
pub const USER_CX1: &str = "c2000000-0000-0000-0000-000000000001";
pub const USER_CX2: &str = "c2000000-0000-0000-0000-000000000002";
pub const MAIN: &str = "MAIN";
pub const CX1: &str = "CX-1";
pub const CX2: &str = "CX-2";

pub const MUST_FINISH: Duration = Duration::from_secs(25);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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

        let schema = unique("al4");
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
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
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
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
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
// Handler Registration check
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_rate_alerts_handler_is_registered_in_outbox_handlers() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap/outbox_handlers");
    assert!(
        dir.join("rate_alerts.rs").exists(),
        "bootstrap/outbox_handlers/rate_alerts.rs must exist"
    );
    let handler_names: Vec<&str> = outbox_handlers::OUTBOX_HANDLERS
        .iter()
        .map(|_| "handler")
        .collect();
    assert!(
        !handler_names.is_empty(),
        "OUTBOX_HANDLERS generated table must have entries"
    );

    let db = TestDb::world().await;
    let settings = common::settings(&[]);
    let state = AppState::new(Arc::new(settings), db.pool.clone());

    let registry = outbox_handlers::registry(&state).expect("registry");
    let handler = registry
        .handler(RATE_ALERTS_PUBLICATION.event_type)
        .expect("rate_alerts handler must be registered in outbox_handlers table");
    assert_eq!(handler.kind(), &RATE_ALERTS_PUBLICATION);

    let probe_event = OutboxEvent {
        id: unique("evt_probe"),
        seq: 1,
        organization_id: ORG.to_string(),
        event_type: RATE_ALERTS_PUBLICATION.event_type.to_string(),
        destination: RATE_ALERTS_PUBLICATION.destination.to_string(),
        aggregate_type: RATE_ALERTS_PUBLICATION.aggregate_type.to_string(),
        aggregate_id: format!("branch:{MAIN}"),
        idempotency_key: "probe".to_string(),
        payload_redacted: json!({
            "branchId": MAIN,
            "publicationId": Uuid::new_v4().to_string(),
            "version": 1,
        }),
        status: OutboxStatus::Pending,
        attempts: 1,
        created_at: Utc::now(),
    };

    // On scaffold: deliver panics with todo!("AL-4: deliver rate alert outbox event")
    // proving the registered handler is the actual AL-4 RateAlertsHandler.
    let _ = handler.deliver(&probe_event).await;

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 Edge Cases (Tests)
// ─────────────────────────────────────────────────────────────────────────────

/// Row 1: an `ABOVE` alert, cooldown 0, publications v1 below, v2 above, v3 above, v4 below, v5 above
/// Expected: events for v2 and v5 only, episodes 1 and 2.
/// Must-Fix 1: Pins last_publication_version at every publication version (R13).
#[tokio::test]
async fn test_above_alert_cooldown_zero_fires_for_v2_and_v5_only_with_episodes_one_and_two() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_cooldown(0)
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);

    // v2 above target (36.00) -> fires episode 1
    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(db.count_events(alert_id).await, 1);
    let events = db.get_events(alert_id).await;
    assert_eq!(events[0].episode_no, 1);
    assert_eq!(events[0].observed_rate, dec("36.00"));
    assert_eq!(
        db.get_alert(alert_id).await.last_publication_version,
        Some(2),
        "v2 must advance last_publication_version to 2"
    );

    // v3 above target (37.00) -> still satisfied, no crossing, no new event
    let pub_v3 = db
        .publish(MAIN, 3, vec![line("USD", "1", "37.00", "37.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v3)
        .await
        .expect("evaluate v3");

    assert_eq!(db.count_events(alert_id).await, 1);
    assert_eq!(
        db.get_alert(alert_id).await.last_publication_version,
        Some(3),
        "v3 must advance last_publication_version to 3"
    );

    // v4 below target (34.00) -> relation becomes NOT_SATISFIED, no event
    let pub_v4 = db
        .publish(MAIN, 4, vec![line("USD", "1", "34.00", "34.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v4)
        .await
        .expect("evaluate v4");

    assert_eq!(db.count_events(alert_id).await, 1);
    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.relation, "NOT_SATISFIED");
    assert_eq!(
        alert.last_publication_version,
        Some(4),
        "v4 must advance last_publication_version to 4"
    );

    // v5 above target (36.50) -> crossing to SATISFIED, fires episode 2
    let pub_v5 = db
        .publish(MAIN, 5, vec![line("USD", "1", "36.50", "37.00")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v5)
        .await
        .expect("evaluate v5");

    assert_eq!(db.count_events(alert_id).await, 2);
    let events = db.get_events(alert_id).await;
    assert_eq!(events[1].episode_no, 2);
    assert_eq!(events[1].observed_rate, dec("36.50"));
    assert_eq!(
        db.get_alert(alert_id).await.last_publication_version,
        Some(5),
        "v5 must advance last_publication_version to 5"
    );

    let notifications = fake_notify.captured().await;
    assert_eq!(
        notifications.len(),
        2,
        "exactly two notifications sent for two fires"
    );

    db.finish().await;
}

/// Row 2: the v2 row delivered twice
/// Expected: one event, one notification.
/// Must-Fix 1 & M2: Pins last_publication_version = 2; and stronger: evaluates v3 (below),
/// then redelivers v2 (above) and expects no second event (stale version skipped).
#[tokio::test]
async fn test_the_v2_row_delivered_twice_produces_one_event_and_one_notification() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);
    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;

    // First delivery / evaluation of v2
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("first evaluation");

    assert_eq!(db.count_events(alert_id).await, 1);
    assert_eq!(fake_notify.captured().await.len(), 1);
    let alert = db.get_alert(alert_id).await;
    assert_eq!(
        alert.last_publication_version,
        Some(2),
        "v2 updates last_publication_version to 2"
    );

    // Second delivery of the exact same publication v2 (at-least-once redelivery)
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("second evaluation");

    assert_eq!(
        db.count_events(alert_id).await,
        1,
        "second delivery of v2 must be idempotent"
    );
    assert_eq!(
        fake_notify.captured().await.len(),
        1,
        "no duplicate notification on redelivery"
    );
    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.last_publication_version, Some(2));

    // Stronger test killing M2: evaluate v3 (below target 35.00 -> relation becomes NOT_SATISFIED)
    let pub_v3 = db
        .publish(MAIN, 3, vec![line("USD", "1", "34.00", "34.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v3)
        .await
        .expect("evaluate v3");

    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.relation, "NOT_SATISFIED");
    assert_eq!(alert.last_publication_version, Some(3));
    assert_eq!(db.count_events(alert_id).await, 1);
    assert_eq!(fake_notify.captured().await.len(), 1);

    // Redeliver stale v2 (which has 36.00 > target 35.00):
    // Because last_publication_version is already 3, lock_due_in skips v2 (< 2 is false).
    // No second event is written, no notification is sent, and relation stays NOT_SATISFIED.
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("redeliver stale v2");

    let alert = db.get_alert(alert_id).await;
    assert_eq!(
        alert.relation, "NOT_SATISFIED",
        "redelivering stale v2 must not restore SATISFIED"
    );
    assert_eq!(
        alert.last_publication_version,
        Some(3),
        "last_publication_version must stay at 3"
    );
    assert_eq!(
        db.count_events(alert_id).await,
        1,
        "stale v2 redelivery produces no new event"
    );
    assert_eq!(
        fake_notify.captured().await.len(),
        1,
        "stale v2 redelivery produces no new notification"
    );

    db.finish().await;
}

/// Row 3: two evaluators on one publication at once
/// Expected: one event (SKIP LOCKED + last_publication_version)
#[tokio::test]
async fn test_two_evaluators_on_one_publication_at_once_produce_one_event() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = Arc::new(RateAlertEvaluator::with_pool(
        db.pool.clone(),
        fake_notify.clone(),
    ));

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);
    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;

    let e1 = evaluator.clone();
    let b1 = branch.clone();
    let t1 = tokio::spawn(async move { e1.evaluate_publication(&b1, pub_v2).await });

    let e2 = evaluator.clone();
    let b2 = branch.clone();
    let t2 = tokio::spawn(async move { e2.evaluate_publication(&b2, pub_v2).await });

    let (r1, r2) = tokio::join!(t1, t2);
    r1.expect("join 1").expect("eval 1");
    r2.expect("join 2").expect("eval 2");

    assert_eq!(
        db.count_events(alert_id).await,
        1,
        "concurrent evaluators must produce exactly one event"
    );
    assert_eq!(
        fake_notify.captured().await.len(),
        1,
        "concurrent evaluators must produce exactly one notification"
    );
    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.last_publication_version, Some(2));

    db.finish().await;
}

/// Row 4: paused at v2, resumed at v3 (above target throughout)
/// Expected: no fire
#[tokio::test]
async fn test_alert_paused_at_v2_and_resumed_at_v3_does_not_fire() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);

    // Alert is paused at v2 (enabled = false)
    db.exec(&format!(
        "UPDATE rate_alerts SET enabled = false WHERE id = '{alert_id}'"
    ))
    .await;

    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(db.count_events(alert_id).await, 0);

    // Resumed at v3 (AL-2 ruling: resume sets relation UNKNOWN and last_publication_version = current version)
    db.exec(&format!(
        "UPDATE rate_alerts SET enabled = true, relation = 'UNKNOWN', \
         last_publication_version = 2 WHERE id = '{alert_id}'"
    ))
    .await;

    let pub_v3 = db
        .publish(MAIN, 3, vec![line("USD", "1", "36.00", "36.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v3)
        .await
        .expect("evaluate v3");

    // UNKNOWN -> SATISFIED is not a crossing from NOT_SATISFIED, so NO fire
    assert_eq!(
        db.count_events(alert_id).await,
        0,
        "resuming into a satisfied rate does not fire"
    );
    assert_eq!(fake_notify.captured().await.len(), 0);

    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.relation, "SATISFIED");
    assert_eq!(alert.last_publication_version, Some(3));

    db.finish().await;
}

/// Row 5: an expired alert
/// Expected: no fire
#[tokio::test]
async fn test_an_expired_alert_does_not_fire() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let expired_at = Utc::now() - chrono::Duration::hours(1);
    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_expires_at(Some(expired_at))
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);
    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(
        db.count_events(alert_id).await,
        0,
        "expired alert must not fire"
    );
    assert_eq!(fake_notify.captured().await.len(), 0);

    db.finish().await;
}

/// Row 6: a line with unit_size != the alert's
/// Expected: disabled UNIT_CHANGED, no event
#[tokio::test]
async fn test_a_line_with_unit_size_different_from_alert_disables_with_unit_changed_no_event() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_currency("JPY")
                .with_unit_size("100")
                .with_target("24.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);
    // Line has unit 1 instead of 100
    let pub_v2 = db
        .publish(MAIN, 2, vec![line("JPY", "1", "0.24", "0.25")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(
        db.count_events(alert_id).await,
        0,
        "unit mismatch writes no event"
    );

    let alert = db.get_alert(alert_id).await;
    assert!(!alert.enabled, "alert must be disabled");
    assert_eq!(
        alert.disabled_reason.as_deref(),
        Some("UNIT_CHANGED"),
        "disabled_reason must be UNIT_CHANGED"
    );

    db.finish().await;
}

/// Row 7: ALL_FRESH, a stale side
/// Expected: skipped, relation unchanged
#[tokio::test]
async fn test_all_fresh_policy_skips_stale_side_leaving_relation_unchanged() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    db.settings(86400, 10, Some(30), "ALL_FRESH").await;

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);
    // Stale side: buy rate 36.00 is above target, but side is stale
    let pub_v2 = db
        .publish(MAIN, 2, vec![stale_line("USD", "1", "36.00", "36.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(
        db.count_events(alert_id).await,
        0,
        "stale side under ALL_FRESH must not fire"
    );

    let alert = db.get_alert(alert_id).await;
    assert_eq!(
        alert.relation, "NOT_SATISFIED",
        "relation must remain unchanged when side is skipped"
    );
    assert_eq!(alert.last_publication_version, Some(2));

    db.finish().await;
}

/// Row 8: ALL_FRESH, a fresh MANUAL side
/// Expected: compared like a fresh AUTO side
#[tokio::test]
async fn test_all_fresh_policy_compares_fresh_manual_side_like_fresh_auto_side() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    db.settings(86400, 10, Some(30), "ALL_FRESH").await;

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);
    // Fresh MANUAL line
    let pub_v2 = db
        .publish(MAIN, 2, vec![manual_line("USD", "1", "36.00", "36.50")])
        .await;
    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(
        db.count_events(alert_id).await,
        1,
        "fresh MANUAL side under ALL_FRESH must be compared and fire"
    );
    let events = db.get_events(alert_id).await;
    assert_eq!(events[0].episode_no, 1);
    assert_eq!(events[0].observed_rate, dec("36.00"));

    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.relation, "SATISFIED");
    assert_eq!(alert.last_publication_version, Some(2));

    db.finish().await;
}

/// Row 9: the notification's params
/// Expected: no customer name, phone or transaction amount (customer-app P10);
/// NotificationObject carries reference ("rate_alert_event", event id);
/// params hold non-numeric display values only (currency, side, relation). No digit run!
#[tokio::test]
async fn test_notification_params_hold_no_customer_name_phone_or_transaction_amount() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;
    evaluator
        .evaluate_publication(&BranchId::new(MAIN), pub_v2)
        .await
        .expect("evaluate v2");

    let notifications = fake_notify.captured().await;
    assert_eq!(notifications.len(), 1);
    let notif = &notifications[0];

    assert_eq!(notif.recipient_user_id(), USER_CX1);
    assert_eq!(notif.template_key(), "rate_alert_crossed");
    let events = db.get_events(alert_id).await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(notif.event_id(), format!("rate_alert:{}", event.id));

    // Must-Fix 5: Reference belongs in NotificationObject
    let object = notif
        .object()
        .expect("NotificationObject must be present on rate alert notification");
    assert_eq!(object.object_type, "rate_alert_event");
    assert_eq!(object.object_id, event.id.to_string());

    let params = notif.params();
    // Non-numeric display values only per lane controller ruling (issuecomment-6119134353)
    assert_eq!(params.get("currency").map(|s| s.as_str()), Some("USD"));
    // Lane controller ruling (issuecomment-6119134353, issuecomment-6119903278):
    // params carry side in customer words (CUSTOMER_SELL for shop Side::Buy, per CustomerSide::to_shop)
    assert_eq!(
        params.get("side").map(|s| s.as_str()),
        Some("CUSTOMER_SELL")
    );
    assert!(
        params.contains_key("relation") || params.contains_key("condition"),
        "params must have relation or condition"
    );

    // Negative checks: MUST NOT hold customer name, phone, or transaction amount (P10)
    // and MUST NOT hold any digit run (no rates, amounts, or timestamps in params)
    for (k, v) in params {
        assert!(
            !v.chars().any(|c| c.is_ascii_digit()),
            "param {k}='{v}' must not contain any digits per ruling (no rates, amounts, or timestamps in params)"
        );
        assert!(
            !v.to_lowercase().contains("somchai"),
            "param {k} contains customer name: {v}"
        );
        let redacted = xc_kernel::pii::redact_text(v);
        assert_eq!(
            redacted, *v,
            "param {k} fails PII redaction check: original '{v}' -> '{redacted}'"
        );
    }

    // The figures stay on the event row (trigger_at, observed_rate)
    assert_eq!(event.observed_rate, dec("36.00"));

    db.finish().await;
}

/// Row 10: a rollback inside the evaluation
/// Expected: no event, no notification, alert unchanged
#[tokio::test]
async fn test_a_rollback_inside_the_evaluation_leaves_alert_unchanged_no_event_no_notification() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    // Simulate notification failure inside the evaluation transaction
    fake_notify.fail_next();

    let branch = BranchId::new(MAIN);
    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;

    let res = evaluator.evaluate_publication(&branch, pub_v2).await;
    assert!(res.is_err(), "evaluation must fail on notification error");

    assert_eq!(
        db.count_events(alert_id).await,
        0,
        "event must be rolled back"
    );

    let alert = db.get_alert(alert_id).await;
    assert_eq!(
        alert.relation, "NOT_SATISFIED",
        "relation must remain NOT_SATISFIED"
    );
    assert_eq!(
        alert.last_publication_version,
        Some(1),
        "last_publication_version must not be updated"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Must-Fix Tests (Review #1009 claude/14cc24bd & Lane Controller claude/ad05bf8d)
// ─────────────────────────────────────────────────────────────────────────────

/// Must-Fix 2: Test the outbox handler end to end (#1009 §11 step 4, §8 input {branchId, publicationId, version})
/// Drives RateAlertsHandler::deliver with an OutboxEvent of kind rate_alerts.publication.
/// One event plus one notification; redelivering the same row is a no-op;
/// Asserts handler(state) is Some and kind() is RATE_ALERTS_PUBLICATION.
#[tokio::test]
async fn test_outbox_handler_end_to_end_delivers_publication_and_is_idempotent() {
    let db = TestDb::world().await;
    let settings = common::settings(&[]);
    let state = AppState::new(Arc::new(settings), db.pool.clone());

    let registry = outbox_handlers::registry(&state).expect("registry");
    let handler = registry
        .handler(RATE_ALERTS_PUBLICATION.event_type)
        .expect("handler(state) is Some");
    assert_eq!(handler.kind(), &RATE_ALERTS_PUBLICATION);

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;

    let event = OutboxEvent {
        id: unique("evt"),
        seq: 1,
        organization_id: ORG.to_string(),
        event_type: RATE_ALERTS_PUBLICATION.event_type.to_string(),
        destination: RATE_ALERTS_PUBLICATION.destination.to_string(),
        aggregate_type: RATE_ALERTS_PUBLICATION.aggregate_type.to_string(),
        aggregate_id: format!("branch:{MAIN}"),
        idempotency_key: pub_v2.to_string(),
        payload_redacted: json!({
            "branchId": MAIN,
            "publicationId": pub_v2.to_string(),
            "version": 2,
        }),
        status: OutboxStatus::Pending,
        attempts: 1,
        created_at: Utc::now(),
    };

    // First delivery: one event and one notification written
    let result = handler.deliver(&event).await;
    match result {
        DeliveryResult::Final(FinalOutcome::Delivered { .. }) => {}
        other => panic!("expected Delivered, got {other:?}"),
    }

    assert_eq!(db.count_events(alert_id).await, 1);
    let events = db.get_events(alert_id).await;
    assert_eq!(events[0].episode_no, 1);
    let event_id_str = format!("rate_alert:{}", events[0].id);
    assert_eq!(db.count_notifications(&event_id_str).await, 1);

    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.last_publication_version, Some(2));

    // Redelivery of the same outbox row is a no-op
    let result2 = handler.deliver(&event).await;
    match result2 {
        DeliveryResult::Final(FinalOutcome::Delivered { .. }) => {}
        other => panic!("expected Delivered on redelivery, got {other:?}"),
    }

    assert_eq!(
        db.count_events(alert_id).await,
        1,
        "idempotent: no duplicate event"
    );
    assert_eq!(
        db.count_notifications(&event_id_str).await,
        1,
        "idempotent: no duplicate notification"
    );

    db.finish().await;
}

/// Must-Fix 3: Version order (design §8 item 3) and Mutant M8 kill
/// Kills Mutant M8: if v2 is delivered when branch current publication is already v3,
/// evaluator evaluates v2's publication (not branch's current publication v3).
/// Also tests that if higher version (v5) is delivered first, lower version (v4) is skipped.
#[tokio::test]
async fn test_version_order_outbox_rows_higher_version_first_skips_lower_version() {
    let db = TestDb::world().await;
    let settings = common::settings(&[]);
    let state = AppState::new(Arc::new(settings), db.pool.clone());

    let registry = outbox_handlers::registry(&state).expect("registry");
    let handler = registry
        .handler(RATE_ALERTS_PUBLICATION.event_type)
        .expect("handler(state) is Some");

    // Case 1: Kill Mutant M8 (evaluate branch current publication instead of event payload publication)
    // Alert 1 watches MAIN with target 35.00
    let alert_id_1 = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(1)),
        )
        .await;

    // Two publications on MAIN: v2 (36.00) and v3 (37.00), both above target.
    // After publishing v3, branches.current_publication_id is v3.
    let pub_v2 = db
        .publish(MAIN, 2, vec![line("USD", "1", "36.00", "36.50")])
        .await;
    let pub_v3 = db
        .publish(MAIN, 3, vec![line("USD", "1", "37.00", "37.50")])
        .await;

    let line_v2_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM published_rate_lines WHERE publication_id = $1 AND currency_code = 'USD'",
    )
    .bind(pub_v2)
    .fetch_one(&db.pool)
    .await
    .expect("fetch v2 line id");

    let event_v2 = OutboxEvent {
        id: unique("evt_v2"),
        seq: 1,
        organization_id: ORG.to_string(),
        event_type: RATE_ALERTS_PUBLICATION.event_type.to_string(),
        destination: RATE_ALERTS_PUBLICATION.destination.to_string(),
        aggregate_type: RATE_ALERTS_PUBLICATION.aggregate_type.to_string(),
        aggregate_id: format!("branch:{MAIN}"),
        idempotency_key: pub_v2.to_string(),
        payload_redacted: json!({
            "branchId": MAIN,
            "publicationId": pub_v2.to_string(),
            "version": 2,
        }),
        status: OutboxStatus::Pending,
        attempts: 1,
        created_at: Utc::now(),
    };

    let event_v3 = OutboxEvent {
        id: unique("evt_v3"),
        seq: 2,
        organization_id: ORG.to_string(),
        event_type: RATE_ALERTS_PUBLICATION.event_type.to_string(),
        destination: RATE_ALERTS_PUBLICATION.destination.to_string(),
        aggregate_type: RATE_ALERTS_PUBLICATION.aggregate_type.to_string(),
        aggregate_id: format!("branch:{MAIN}"),
        idempotency_key: pub_v3.to_string(),
        payload_redacted: json!({
            "branchId": MAIN,
            "publicationId": pub_v3.to_string(),
            "version": 3,
        }),
        status: OutboxStatus::Pending,
        attempts: 1,
        created_at: Utc::now(),
    };

    // Deliver v2's row FIRST (while branches.current_publication_id in DB is v3):
    let r2 = handler.deliver(&event_v2).await;
    match r2 {
        DeliveryResult::Final(FinalOutcome::Delivered { .. }) => {}
        other => panic!("expected Delivered for v2, got {other:?}"),
    }

    assert_eq!(db.count_events(alert_id_1).await, 1);
    let events = db.get_events(alert_id_1).await;
    assert_eq!(events[0].episode_no, 1);
    // Kills Mutant M8: event must record v2's line and observed rate 36.00,
    // NEVER the branch's current publication line or 37.00.
    assert_eq!(
        events[0].published_rate_line_id, line_v2_id,
        "event published_rate_line_id must be v2's line, not branch current_publication_id (v3)"
    );
    assert_eq!(
        events[0].observed_rate,
        dec("36.00"),
        "event observed_rate must be v2's 36.00, not branch current publication v3's 37.00"
    );
    let alert1 = db.get_alert(alert_id_1).await;
    assert_eq!(
        alert1.last_publication_version,
        Some(2),
        "last_publication_version must be 2 after v2 delivery (not 3)"
    );
    assert_eq!(alert1.relation, "SATISFIED");

    // Deliver v3 second:
    let r3 = handler.deliver(&event_v3).await;
    match r3 {
        DeliveryResult::Final(FinalOutcome::Delivered { .. }) => {}
        other => panic!("expected Delivered for v3, got {other:?}"),
    }

    // Event count remains 1 because rate is still SATISFIED (no new crossing)
    assert_eq!(db.count_events(alert_id_1).await, 1);
    let alert1 = db.get_alert(alert_id_1).await;
    assert_eq!(
        alert1.last_publication_version,
        Some(3),
        "last_publication_version advances to 3"
    );

    // Case 2: Outbox rows arriving out-of-order: higher version delivered first skips lower version (design §8 item 3)
    let alert_id_2 = db
        .insert_alert(
            &AlertBuilder::new()
                .with_target("38.00")
                .with_relation("NOT_SATISFIED")
                .with_last_version(Some(3)),
        )
        .await;

    let pub_v4 = db
        .publish(MAIN, 4, vec![line("USD", "1", "39.00", "39.50")])
        .await;
    let pub_v5 = db
        .publish(MAIN, 5, vec![line("USD", "1", "40.00", "40.50")])
        .await;

    let event_v4 = OutboxEvent {
        id: unique("evt_v4"),
        seq: 4,
        organization_id: ORG.to_string(),
        event_type: RATE_ALERTS_PUBLICATION.event_type.to_string(),
        destination: RATE_ALERTS_PUBLICATION.destination.to_string(),
        aggregate_type: RATE_ALERTS_PUBLICATION.aggregate_type.to_string(),
        aggregate_id: format!("branch:{MAIN}"),
        idempotency_key: pub_v4.to_string(),
        payload_redacted: json!({
            "branchId": MAIN,
            "publicationId": pub_v4.to_string(),
            "version": 4,
        }),
        status: OutboxStatus::Pending,
        attempts: 1,
        created_at: Utc::now(),
    };

    let event_v5 = OutboxEvent {
        id: unique("evt_v5"),
        seq: 5,
        organization_id: ORG.to_string(),
        event_type: RATE_ALERTS_PUBLICATION.event_type.to_string(),
        destination: RATE_ALERTS_PUBLICATION.destination.to_string(),
        aggregate_type: RATE_ALERTS_PUBLICATION.aggregate_type.to_string(),
        aggregate_id: format!("branch:{MAIN}"),
        idempotency_key: pub_v5.to_string(),
        payload_redacted: json!({
            "branchId": MAIN,
            "publicationId": pub_v5.to_string(),
            "version": 5,
        }),
        status: OutboxStatus::Pending,
        attempts: 1,
        created_at: Utc::now(),
    };

    // If higher version (v5) is delivered first:
    let r5 = handler.deliver(&event_v5).await;
    match r5 {
        DeliveryResult::Final(FinalOutcome::Delivered { .. }) => {}
        other => panic!("expected Delivered for v5, got {other:?}"),
    }

    assert_eq!(db.count_events(alert_id_2).await, 1);
    let alert2 = db.get_alert(alert_id_2).await;
    assert_eq!(alert2.last_publication_version, Some(5));
    assert_eq!(alert2.relation, "SATISFIED");

    // Then lower version (v4) is delivered second: v4 is skipped because last_publication_version is already 5
    let r4 = handler.deliver(&event_v4).await;
    match r4 {
        DeliveryResult::Final(FinalOutcome::Delivered { .. }) => {}
        other => panic!("expected Delivered for v4, got {other:?}"),
    }

    assert_eq!(
        db.count_events(alert_id_2).await,
        1,
        "v4 skipped: no second event"
    );
    let alert2 = db.get_alert(alert_id_2).await;
    assert_eq!(
        alert2.last_publication_version,
        Some(5),
        "last_publication_version remains 5"
    );
    assert_eq!(
        alert2.relation, "SATISFIED",
        "relation unchanged by skipped v4"
    );

    db.finish().await;
}

/// Must-Fix 5: Template test for RateAlertCrossedTemplate
/// key rate_alert_crossed, Transactional, TH/EN/ZH, params hold NO digit run
/// (only currency code, customer-words side and ABOVE/BELOW), and NotificationObject = (rate_alert_event, event id).
#[test]
fn test_rate_alert_crossed_template_metadata_and_rendering_th_en_zh() {
    let template = RateAlertCrossedTemplate;
    assert_eq!(template.key(), "rate_alert_crossed");
    assert_eq!(template.kind(), TemplateKind::Transactional);
    assert_eq!(template.version(), 1);

    let mut params = std::collections::BTreeMap::new();
    params.insert("currency".to_string(), "USD".to_string());
    params.insert("side".to_string(), "CUSTOMER_SELL".to_string());
    params.insert("relation".to_string(), "ABOVE".to_string());

    // Verify params hold NO digit run (ruling: only currency code, customer-words side, and relation)
    for (k, v) in &params {
        assert!(
            !v.chars().any(|c| c.is_ascii_digit()),
            "param {k}='{v}' must not contain any digits"
        );
        let redacted = xc_kernel::pii::redact_text(v);
        assert_eq!(
            redacted, *v,
            "param {k}='{v}' must pass PII redaction without change"
        );
    }

    // On scaffold / main: render will fail by result with todo!("AL-4: render rate_alert_crossed template")
    for locale in [Locale::Th, Locale::En, Locale::Zh] {
        let rendered = template.render(locale, &params);
        assert!(
            !rendered.title.trim().is_empty(),
            "rendered title for {:?} must not be empty",
            locale
        );
        assert!(
            !rendered.body.trim().is_empty(),
            "rendered body for {:?} must not be empty",
            locale
        );
        // Template text has no figures / digits per lane controller ruling
        assert!(
            !rendered.title.chars().any(|c| c.is_ascii_digit()),
            "title for {:?} must contain no figures: {}",
            locale,
            rendered.title
        );
        assert!(
            !rendered.body.chars().any(|c| c.is_ascii_digit()),
            "body for {:?} must contain no figures: {}",
            locale,
            rendered.body
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Ticket #1653: Rate-alert evaluator denomination_group = 'ALL' (§9 rows 1–3)
// ─────────────────────────────────────────────────────────────────────────────

/// §9 Row 1 (#1653): A publication with USD/ALL and USD/<another group> at different rates;
/// the alert's target is crossed only by the ALL rate.
/// Required result: fires once, and published_rate_line_id is the ALL line.
#[tokio::test]
async fn test_publication_with_all_and_other_group_crossed_by_all_fires_once_with_all_line_id() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_currency("USD")
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_cooldown(0)
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);

    // ALL line rate 36.00 crosses target 35.00; NOTE_50_100 line rate 34.00 does not cross
    let (pub_v2, line_ids) = db
        .publish_with_groups(
            MAIN,
            2,
            vec![
                ("ALL", line("USD", "1", "36.00", "36.50")),
                ("NOTE_50_100", line("USD", "1", "34.00", "34.50")),
            ],
        )
        .await;
    let all_line_id = line_ids[0];

    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(db.count_events(alert_id).await, 1, "alert must fire once");
    let events = db.get_events(alert_id).await;
    assert_eq!(
        events[0].published_rate_line_id, all_line_id,
        "published_rate_line_id must be the ALL line"
    );
    assert_eq!(events[0].observed_rate, dec("36.00"));
    assert_eq!(events[0].episode_no, 1);

    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.relation, "SATISFIED");
    assert_eq!(alert.last_publication_version, Some(2));

    let notifications = fake_notify.captured().await;
    assert_eq!(notifications.len(), 1, "exactly one notification sent");

    db.finish().await;
}

/// §9 Row 2 (#1653): The same, but crossed only by the other group's rate.
/// Required result: does not fire.
#[tokio::test]
async fn test_publication_with_all_and_other_group_crossed_only_by_other_group_does_not_fire() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_currency("USD")
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_cooldown(0)
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);

    // ALL line rate 34.00 does not cross target 35.00; NOTE_50_100 line rate 36.00 crosses
    let (pub_v2, _line_ids) = db
        .publish_with_groups(
            MAIN,
            2,
            vec![
                ("ALL", line("USD", "1", "34.00", "34.50")),
                ("NOTE_50_100", line("USD", "1", "36.00", "36.50")),
            ],
        )
        .await;

    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    assert_eq!(
        db.count_events(alert_id).await,
        0,
        "must not fire when crossed only by other denomination group"
    );
    let alert = db.get_alert(alert_id).await;
    assert_eq!(alert.relation, "NOT_SATISFIED");
    assert_eq!(alert.last_publication_version, Some(2));

    let notifications = fake_notify.captured().await;
    assert_eq!(notifications.len(), 0, "no notification sent");

    db.finish().await;
}

/// §9 Row 3 (#1653): The other group's unit_size differs from the alert's; ALL's matches.
/// Required result: the alert stays enabled (no UNIT_CHANGED).
#[tokio::test]
async fn test_other_group_unit_size_differs_from_alert_all_matches_alert_stays_enabled() {
    let db = TestDb::world().await;
    let fake_notify = Arc::new(FakeNotifications::new());
    let evaluator = RateAlertEvaluator::with_pool(db.pool.clone(), fake_notify.clone());

    let alert_id = db
        .insert_alert(
            &AlertBuilder::new()
                .with_currency("USD")
                .with_unit_size("1")
                .with_target("35.00")
                .with_relation("NOT_SATISFIED")
                .with_cooldown(0)
                .with_last_version(Some(1)),
        )
        .await;

    let branch = BranchId::new(MAIN);

    // ALL line unit_size is 1 (matches alert); NOTE_50_100 line unit_size is 100 (differs)
    let (pub_v2, _line_ids) = db
        .publish_with_groups(
            MAIN,
            2,
            vec![
                ("ALL", line("USD", "1", "34.00", "34.50")),
                ("NOTE_50_100", line("USD", "100", "3400.00", "3450.00")),
            ],
        )
        .await;

    evaluator
        .evaluate_publication(&branch, pub_v2)
        .await
        .expect("evaluate v2");

    let alert = db.get_alert(alert_id).await;
    assert!(
        alert.enabled,
        "alert must stay enabled when ALL matches unit_size"
    );
    assert_eq!(
        alert.disabled_reason, None,
        "no UNIT_CHANGED disabled_reason when ALL matches unit_size"
    );
    assert_eq!(alert.last_publication_version, Some(2));

    db.finish().await;
}
