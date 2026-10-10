//! APP-3 (#993, design `docs/design/blueprint/customer-app.md` P8–P10, P15, §4.1–§4.3, §9.1) —
//! the database harness, the seeds and two test templates. Written before the implementation by
//! the test author (claude/dell-claude-2, for the ticket author). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose
//! name ends in `_test`; without it every DB test FAILS, none skips. Each `TestDb` is a schema of
//! its own with `api/migrations/` applied (APP-1's `20261106100000_customer_app.sql` holds the
//! tables). SQL must name tables without a schema prefix.
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;

pub use xc_kernel::notifications::{
    NotificationId, NotificationObject, NotificationRequest, NotificationRequestError,
    Notifications,
};
pub use xc_notify::application::notification_service::{
    MyNotifications, MyNotificationsQuery, NotificationService,
};
pub use xc_notify::application::template_registry::{DuplicateTemplateKey, TemplateRegistry};
pub use xc_notify::domain::error::NotifyError;
pub use xc_notify::domain::notification::NotificationItem;
pub use xc_notify::domain::template::{Locale, NotificationTemplate, Rendered, TemplateKind};

pub const MUST_FINISH: Duration = Duration::from_secs(30);
/// The organisation every v3 database has (`20261011100000_organizations.sql:9-10`).
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";

pub const TRANSACTIONAL: &str = "test_transactional";
pub const TRANSACTIONAL_VERSION: i32 = 3;
pub const MARKETING: &str = "test_marketing";

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

/// A transactional template whose title and body differ in every locale.
pub struct TestTransactional;

impl NotificationTemplate for TestTransactional {
    fn key(&self) -> &'static str {
        TRANSACTIONAL
    }
    fn version(&self) -> i32 {
        TRANSACTIONAL_VERSION
    }
    fn kind(&self) -> TemplateKind {
        TemplateKind::Transactional
    }
    fn render(&self, locale: Locale, params: &BTreeMap<String, String>) -> Rendered {
        let n = params.get("displayNumber").cloned().unwrap_or_default();
        match locale {
            Locale::Th => Rendered {
                title: "ถึงคิวของคุณแล้ว".into(),
                body: format!("คิว {n}"),
            },
            Locale::En => Rendered {
                title: "It is your turn".into(),
                body: format!("Queue {n}"),
            },
            Locale::Zh => Rendered {
                title: "轮到您了".into(),
                body: format!("排队号 {n}"),
            },
        }
    }
}

/// A marketing template: it needs current marketing consent.
pub struct TestMarketing;

impl NotificationTemplate for TestMarketing {
    fn key(&self) -> &'static str {
        MARKETING
    }
    fn version(&self) -> i32 {
        1
    }
    fn kind(&self) -> TemplateKind {
        TemplateKind::Marketing
    }
    fn render(&self, locale: Locale, _params: &BTreeMap<String, String>) -> Rendered {
        Rendered {
            title: format!("promo-{}", locale.as_str()),
            body: "promo".into(),
        }
    }
}

pub fn registry() -> Arc<TemplateRegistry> {
    Arc::new(
        TemplateRegistry::from_templates(vec![
            Box::new(TestTransactional),
            Box::new(TestMarketing),
        ])
        .expect("two templates with two keys"),
    )
}

pub fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

pub fn request(user: &str, event: &str, template: &str) -> NotificationRequest {
    NotificationRequest::new(
        user,
        event,
        template,
        Some(NotificationObject {
            object_type: "queue_ticket".into(),
            object_id: "T-1".into(),
        }),
        params(&[("displayNumber", "W001")]),
    )
    .expect("a valid request")
}

/// One `notification_deliveries` row, read back by the tests.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DeliveryRow {
    pub channel: String,
    pub status: String,
    pub skipped_reason: Option<String>,
    pub organization_id: String,
}

/// One `notifications` row, read back by the tests.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct NotificationRow {
    pub recipient_user_id: String,
    pub event_id: String,
    pub template_key: String,
    pub template_version: i32,
    pub locale: String,
    pub object_type: Option<String>,
    pub object_id: Option<String>,
    pub organization_id: String,
    pub read_at: Option<DateTime<Utc>>,
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The xc-notify tests need a real PostgreSQL and do \
                 not skip, e.g. TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );

        let schema = unique("app3");
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
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Self { pool, schema }
    }

    pub fn service(&self) -> NotificationService {
        NotificationService::new(self.pool.clone(), registry())
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin")
    }

    /// A signed-in user of the organisation; answers its id.
    pub async fn seed_user(&self) -> String {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("user"))
        .fetch_one(&self.pool)
        .await
        .expect("insert into users");
        id
    }

    /// A user who is a customer with `preferred_language`; answers (user id, customer id).
    pub async fn seed_customer_user(&self, preferred_language: &str) -> (String, String) {
        let user = self.seed_user().await;
        let customer = unique("C");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, \
                 user_id, preferred_language) \
             VALUES ($1, $2::uuid, $1, $1, 'LIVE', $3, $4::uuid, $5)",
        )
        .bind(&customer)
        .bind(ORG)
        .bind(at(0))
        .bind(&user)
        .bind(preferred_language)
        .execute(&self.pool)
        .await
        .expect("insert into customers");
        (user, customer)
    }

    pub async fn set_preferred_language(&self, customer: &str, lang: &str) {
        sqlx::query("UPDATE customers SET preferred_language = $2 WHERE id = $1")
            .bind(customer)
            .bind(lang)
            .execute(&self.pool)
            .await
            .expect("update customers.preferred_language");
    }

    /// One `consents` row for MARKETING on `channel` (append-only: a withdrawal is a later row).
    pub async fn record_marketing(&self, customer: &str, channel: &str, granted: bool, when: i64) {
        sqlx::query(
            "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, \
                 lang, granted, effective_at, evidence_ref) \
             VALUES ($1::uuid, $2::uuid, $3, 'MARKETING', $4, 1, 'th', $5, $6, 'test')",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(ORG)
        .bind(customer)
        .bind(channel)
        .bind(granted)
        .bind(at(when))
        .execute(&self.pool)
        .await
        .expect("insert into consents");
    }

    /// Notify in a transaction of its own, committed.
    pub async fn notify(
        &self,
        service: &NotificationService,
        req: &NotificationRequest,
    ) -> NotificationId {
        let mut tx = self.begin().await;
        let id = service.notify_in(&mut tx, req).await.expect("notify_in");
        tx.commit().await.expect("commit");
        id
    }

    pub async fn notification_count(&self, user: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE recipient_user_id = $1::uuid")
            .bind(user)
            .fetch_one(&self.pool)
            .await
            .expect("count notifications")
    }

    pub async fn delivery_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM notification_deliveries")
            .fetch_one(&self.pool)
            .await
            .expect("count deliveries")
    }

    pub async fn notification(&self, id: &NotificationId) -> NotificationRow {
        sqlx::query_as(
            "SELECT recipient_user_id::text, event_id, template_key, template_version, locale, object_type, \
                 object_id, organization_id::text, read_at \
             FROM notifications WHERE id = $1::uuid",
        )
        .bind(&id.0)
        .fetch_one(&self.pool)
        .await
        .expect("read the notification")
    }

    pub async fn deliveries(&self, id: &NotificationId) -> Vec<DeliveryRow> {
        sqlx::query_as(
            "SELECT channel, status, skipped_reason, organization_id::text FROM notification_deliveries \
             WHERE notification_id = $1::uuid ORDER BY channel",
        )
        .bind(&id.0)
        .fetch_all(&self.pool)
        .await
        .expect("read the deliveries")
    }

    pub async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}
