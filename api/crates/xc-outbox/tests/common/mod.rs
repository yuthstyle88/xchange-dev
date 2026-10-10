//! Ticket OB-1 (#563 design `docs/design/blueprint/broker-seam.md` §5, §11) — the database
//! harness and the fake handler of the outbox-core tests. Written before the implementation by
//! the ticket author. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose
//! name ends in `_test`. When the variable is absent every DB test FAILS; none of them skips.
//! Each `TestDb` is a schema of its own with `api/migrations/` applied (the outbox migration,
//! `20261103100000_outbox.sql`, is the implementer's), so tests run in parallel and the
//! migration runs from an empty schema every time. SQL in the migration and the implementation
//! must therefore name tables without a schema prefix.
#![allow(dead_code, unused_imports)]

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;

pub use xc_outbox::application::registry::OutboxRegistry;
pub use xc_outbox::application::worker::{LEASE, OutboxWorker, TickReport};
pub use xc_outbox::application::writer::OutboxWriter;
pub use xc_outbox::domain::event::{OutboxEvent, OutboxStatus};
pub use xc_outbox::domain::event_kind::EventKind;
pub use xc_outbox::domain::handler::{DeliveryResult, FinalOutcome, OutboxHandler};
pub use xc_outbox::domain::retry_policy::RetryPolicy;
pub use xc_outbox::domain::store::OutboxStore;
pub use xc_outbox::infrastructure::pg_store::PgOutboxStore;

pub const MUST_FINISH: Duration = Duration::from_secs(30);

/// The organisation every v3 database has (`20261011100000_organizations.sql:9-10`).
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// A second organisation, inserted by `TestDb::second_org`.
pub const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";

/// An earn and a reversal for the same destination: FIFO is per destination, not per type.
pub static EARN: EventKind = EventKind {
    event_type: "test.loyalty.earn",
    destination: "loyalty",
    aggregate_type: "transaction",
};
pub static REVERSE: EventKind = EventKind {
    event_type: "test.loyalty.reverse",
    destination: "loyalty",
    aggregate_type: "transaction",
};
/// The same business event for another destination (fan-out, design §5.2).
pub static GL: EventKind = EventKind {
    event_type: "test.gl",
    destination: "accountzing",
    aggregate_type: "transaction",
};
/// A kind no handler is registered for.
pub static ORPHAN: EventKind = EventKind {
    event_type: "test.orphan",
    destination: "nowhere",
    aggregate_type: "transaction",
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

pub fn key() -> String {
    unique("key")
}

pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

/// `true` when `result` is the conflict the writer answers for a reused key.
pub fn is_idempotency_conflict<T>(result: &AppResult<T>) -> bool {
    matches!(result, Err(AppError::Conflict(code)) if code == "idempotencyConflict")
}

// ── the fake handler ────────────────────────────────────────────────────────

/// One delivery the fake handler saw: `(event id, idempotency key, attempts)`.
pub type Seen = (String, String, i32);

/// A handler whose answer per event id is scripted by the test. With nothing scripted it
/// answers `Final(Delivered { 201 })`. Every `deliver` and `on_final` is recorded.
pub struct FakeHandler {
    kind: &'static EventKind,
    policy: Option<RetryPolicy>,
    script: Mutex<HashMap<String, VecDeque<DeliveryResult>>>,
    seen: Mutex<Vec<Seen>>,
    finals: Mutex<Vec<(String, FinalOutcome)>>,
    fail_on_final: AtomicBool,
}

impl FakeHandler {
    pub fn new(kind: &'static EventKind) -> Arc<Self> {
        Self::with_policy(kind, None)
    }

    pub fn with_policy(kind: &'static EventKind, policy: Option<RetryPolicy>) -> Arc<Self> {
        Arc::new(Self {
            kind,
            policy,
            script: Mutex::new(HashMap::new()),
            seen: Mutex::new(Vec::new()),
            finals: Mutex::new(Vec::new()),
            fail_on_final: AtomicBool::new(false),
        })
    }

    /// The next answers for `event_id`, in order; after them, `Delivered { 201 }`.
    pub fn answer(&self, event_id: &str, answers: Vec<DeliveryResult>) {
        self.script
            .lock()
            .expect("not poisoned")
            .insert(event_id.to_owned(), answers.into());
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("not poisoned").clone()
    }

    /// How many times `event_id` was delivered.
    pub fn deliveries_of(&self, event_id: &str) -> usize {
        self.seen()
            .iter()
            .filter(|(id, _, _)| id == event_id)
            .count()
    }

    pub fn finals(&self) -> Vec<(String, FinalOutcome)> {
        self.finals.lock().expect("not poisoned").clone()
    }

    /// From now on `on_final` answers an error.
    pub fn fail_on_final(&self, fail: bool) {
        self.fail_on_final.store(fail, Ordering::SeqCst);
    }
}

impl OutboxHandler for FakeHandler {
    fn kind(&self) -> &'static EventKind {
        self.kind
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.policy.unwrap_or_default()
    }

    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, DeliveryResult> {
        Box::pin(async move {
            self.seen.lock().expect("not poisoned").push((
                event.id.clone(),
                event.idempotency_key.clone(),
                event.attempts,
            ));
            self.script
                .lock()
                .expect("not poisoned")
                .get_mut(&event.id)
                .and_then(VecDeque::pop_front)
                .unwrap_or(DeliveryResult::Final(FinalOutcome::Delivered {
                    http_status: 201,
                }))
        })
    }

    fn on_final<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a OutboxEvent,
        outcome: &'a FinalOutcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            // Touch the caller's transaction: it must be a live one.
            sqlx::query("SELECT 1").execute(&mut **tx).await?;
            if self.fail_on_final.load(Ordering::SeqCst) {
                return Err(AppError::Internal("on_final refused (test)".to_owned()));
            }
            self.finals
                .lock()
                .expect("not poisoned")
                .push((event.id.clone(), outcome.clone()));
            Ok(())
        })
    }
}

/// `deliver` answers `Retry` with this error, for the backoff tests.
pub fn retry(status: u16) -> DeliveryResult {
    DeliveryResult::Retry {
        http_status: Some(status),
        error: format!("upstream answered {status}"),
    }
}

// ── the database ────────────────────────────────────────────────────────────

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

static STARTING: Semaphore = Semaphore::const_new(6);

/// One row of `outbox_events`, read back by the tests.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Row {
    pub id: String,
    pub organization_id: String,
    pub event_type: String,
    pub destination: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub idempotency_key: String,
    pub payload_hash: String,
    pub payload_redacted: Value,
    pub status: String,
    pub attempts: i32,
    pub last_http_status: Option<i32>,
    pub last_error: Option<String>,
    pub delivered: bool,
    /// `next_attempt_at − now()` in seconds.
    pub due_in_secs: f64,
}

impl TestDb {
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The xc-outbox tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
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

        let schema = unique("ob1");
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
            .max_connections(12)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        Self { pool, schema }
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin a transaction")
    }

    /// Insert `ORG_B` into `organizations`.
    pub async fn second_org(&self) {
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1::uuid, 'OB1B', NULL, 'Asia/Bangkok', now())",
        )
        .bind(ORG_B)
        .execute(&self.pool)
        .await
        .expect("insert the second organisation");
    }

    /// Enqueue one event for `ORG` in a transaction of its own and commit it.
    pub async fn enqueue(
        &self,
        kind: &'static EventKind,
        aggregate_id: &str,
        key: &str,
        payload: Value,
    ) -> String {
        let mut tx = self.begin().await;
        let id = OutboxWriter::enqueue_in(&mut tx, ORG, kind, aggregate_id, key, &payload)
            .await
            .expect("enqueue");
        tx.commit().await.expect("commit the enqueue");
        id
    }

    pub async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM outbox_events")
            .fetch_one(&self.pool)
            .await
            .expect("count outbox_events")
    }

    pub async fn row(&self, id: &str) -> Row {
        sqlx::query_as(
            "SELECT id::text AS id, organization_id::text AS organization_id, event_type, \
                    destination, aggregate_type, aggregate_id, idempotency_key, payload_hash, \
                    payload_redacted, status, attempts, last_http_status, last_error, \
                    delivered_at IS NOT NULL AS delivered, \
                    EXTRACT(EPOCH FROM (next_attempt_at - now()))::float8 AS due_in_secs \
             FROM outbox_events WHERE id::text = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read outbox row {id}: {e}"))
    }

    pub async fn status(&self, id: &str) -> String {
        self.row(id).await.status
    }

    /// Make the row due now (its backoff or lease has run out).
    pub async fn make_due(&self, id: &str) {
        sqlx::query(
            "UPDATE outbox_events SET next_attempt_at = now() - interval '1 second' \
             WHERE id::text = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .expect("make the row due");
    }

    /// Move the row's `created_at` this far into the past.
    pub async fn age(&self, id: &str, by: &str) {
        sqlx::query(AssertSqlSafe(format!(
            "UPDATE outbox_events SET created_at = now() - interval '{by}' WHERE id::text = $1"
        )))
        .bind(id)
        .execute(&self.pool)
        .await
        .expect("age the row");
    }

    pub fn store(&self) -> Arc<PgOutboxStore> {
        Arc::new(PgOutboxStore::new(self.pool.clone()))
    }

    pub fn worker(&self, handlers: Vec<Arc<dyn OutboxHandler>>) -> OutboxWorker {
        let registry = OutboxRegistry::new(handlers).expect("a registry without duplicates");
        OutboxWorker::new(self.store(), Arc::new(registry))
    }

    pub async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect to drop the test schema");
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

/// Tick until a tick claims nothing, at most `max` times.
pub async fn drain(worker: &OutboxWorker, max: usize) {
    for _ in 0..max {
        let report = worker.tick().await.expect("tick");
        if report == TickReport::default() {
            return;
        }
    }
}
