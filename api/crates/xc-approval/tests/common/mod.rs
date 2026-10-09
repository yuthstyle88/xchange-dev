//! Ticket #596 (AP-1) — the database harness and the fake approval types of the
//! approval-core tests. Written before the implementation by the ticket author
//! (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §5 (approvals and audit, #549), §5.6
//! (concurrency and idempotency), §7 (class diagram), §9 (AP-1, up-front tests).
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `TestDb` is a schema of its own with
//! `api/migrations/` applied (migration 2, `20261010110000_approvals.sql`, is
//! the implementer's), so tests run in parallel and the migration is exercised
//! from an empty schema every time. SQL in the migration and in the
//! implementation must therefore name tables without a schema prefix.
//!
//! Signatures this harness assumes (see `approval_rules_test.rs` for the list
//! the tests themselves assume):
//! - `ApprovalSubject::on_decided` takes the caller's transaction as
//!   `&'a mut sqlx::Transaction<'_, sqlx::Postgres>`, the repo's convention for
//!   "in the caller's tx" (`xc-shift/src/domain/custody/repositories/custody_repository.rs:15-36`).
//!   Design §5.2 at `4b8f73b` printed `&'a mut BranchTx`; `BranchTx` opens a
//!   transaction of its own (`xc-kernel/src/branch_tx.rs:55`), so it cannot be
//!   the caller's. The design is corrected to this type (see #596).
//! - `Refusal::new(code: &'static str, message: impl Into<String>)` and
//!   `Outcome::{Approved, Rejected}` with `Clone + Copy + Debug + PartialEq`.
//! - `ApprovalRegistry::new()` and `register(&mut self, Arc<dyn ApprovalSubject>,
//!   Arc<dyn ApprovalPolicy>)`, keyed by the subject's `type_name()`.
//! - `ApprovalService::new(ApprovalRegistry, Arc<dyn ApprovalRepository>,
//!   Arc<dyn AuditLog>)`, `PgApprovalRepository::new(PgPool)`, `PgAuditLog::new(PgPool)`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;

pub use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
pub use xc_approval::application::registry::ApprovalRegistry;
pub use xc_approval::domain::approval_repository::ApprovalRepository;
pub use xc_approval::domain::audit_log::{AuditEntry, AuditEvent, AuditLog};
pub use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
pub use xc_approval::domain::request::{ApprovalRequest, Outcome};
pub use xc_approval::domain::subject::ApprovalSubject;
pub use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
pub use xc_approval::infrastructure::audit_log::PgAuditLog;

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(20);
/// How long something is watched to prove it has not happened.
pub const STAYS_OUT: Duration = Duration::from_millis(400);

pub const TYPE_A: &str = "TEST_A";
pub const TYPE_B: &str = "TEST_B";
pub const POLICY_VERSION: &str = "pv-7";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// An idempotency key, different on every call.
pub fn key() -> String {
    unique("key")
}

/// Await `fut`, failing the test when it takes longer than `limit`.
pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

/// The v3 code of a refusal (§4.7 "Error codes"). Panics on success.
pub fn code<T: Debug>(result: Result<T, ApprovalError>) -> &'static str {
    match result {
        Err(e) => e.code(),
        Ok(value) => panic!("expected a refusal, got Ok({value:?})"),
    }
}

pub fn owner(id: &str) -> Actor {
    Actor::owner(id)
}

pub fn rates(id: &str) -> Actor {
    Actor::staff(id, Role::Rates, BranchId::new("MAIN")).expect("RATES is a staff role")
}

// ── fakes ───────────────────────────────────────────────────────────────────

/// A subject under the test's control: the hash of each object is whatever the
/// test last set, and every `on_decided` call is recorded.
pub struct FakeSubject {
    type_name: &'static str,
    hashes: Mutex<HashMap<String, String>>,
    decided: Mutex<Vec<(String, Outcome)>>,
}

impl FakeSubject {
    pub fn new(type_name: &'static str) -> Arc<Self> {
        Arc::new(Self {
            type_name,
            hashes: Mutex::new(HashMap::new()),
            decided: Mutex::new(Vec::new()),
        })
    }

    /// The object's hash from now on.
    pub fn set_hash(&self, object_id: &str, hash: &str) {
        self.hashes
            .lock()
            .expect("not poisoned")
            .insert(object_id.to_owned(), hash.to_owned());
    }

    /// Every `(object_id, outcome)` `on_decided` was called with, in order.
    pub fn decided(&self) -> Vec<(String, Outcome)> {
        self.decided.lock().expect("not poisoned").clone()
    }
}

impl ApprovalSubject for FakeSubject {
    fn type_name(&self) -> &'static str {
        self.type_name
    }

    fn version_hash<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            Ok(self
                .hashes
                .lock()
                .expect("not poisoned")
                .get(object_id)
                .cloned()
                .unwrap_or_else(|| "h-initial".to_owned()))
        })
    }

    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            // Touch the caller's transaction: it must be a live one.
            sqlx::query("SELECT 1").execute(&mut **tx).await?;
            self.decided
                .lock()
                .expect("not poisoned")
                .push((object_id.to_owned(), outcome));
            Ok(())
        })
    }
}

/// A policy that lets anybody decide, so that only the core can refuse the
/// proposer. Whether a proposer may close their own request is set per test.
pub struct AnyonePolicy {
    pub self_decision: bool,
}

impl ApprovalPolicy for AnyonePolicy {
    fn may_decide(&self, _who: &Actor, _request: &ApprovalRequest) -> Result<(), Refusal> {
        Ok(())
    }

    fn allows_self_decision(&self, _who: &Actor) -> bool {
        self.self_decision
    }

    fn policy_version(&self) -> String {
        POLICY_VERSION.to_owned()
    }
}

/// OWNER decides, nobody else (the shape of `RatePublicationPolicy`, A2/A5).
pub struct OwnerPolicy;

impl ApprovalPolicy for OwnerPolicy {
    fn may_decide(&self, who: &Actor, _request: &ApprovalRequest) -> Result<(), Refusal> {
        if who.role() == Role::Owner {
            Ok(())
        } else {
            Err(Refusal::new("outOfScope", "only OWNER decides this type"))
        }
    }

    fn allows_self_decision(&self, who: &Actor) -> bool {
        who.role() == Role::Owner
    }

    fn policy_version(&self) -> String {
        POLICY_VERSION.to_owned()
    }
}

/// A policy that keeps the trait's default `allows_self_decision` (no).
pub struct DefaultPolicy;

impl ApprovalPolicy for DefaultPolicy {
    fn may_decide(&self, _who: &Actor, _request: &ApprovalRequest) -> Result<(), Refusal> {
        Ok(())
    }

    fn policy_version(&self) -> String {
        POLICY_VERSION.to_owned()
    }
}

// ── the database ────────────────────────────────────────────────────────────

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The xc-approval tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`. \
             Use a dedicated test database."
        );

        let schema = unique("ap1");
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
            .max_connections(10)
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

    /// The core with `subjects` registered, each with its policy.
    pub fn service(
        &self,
        subjects: Vec<(Arc<dyn ApprovalSubject>, Arc<dyn ApprovalPolicy>)>,
    ) -> Arc<ApprovalService> {
        let mut registry = ApprovalRegistry::new();
        for (subject, policy) in subjects {
            registry.register(subject, policy);
        }
        let repository: Arc<dyn ApprovalRepository> =
            Arc::new(PgApprovalRepository::new(self.pool.clone()));
        let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(self.pool.clone()));
        Arc::new(ApprovalService::new(registry, repository, audit))
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin a transaction")
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// `approval_requests.status` of one request (§5.1).
    pub async fn status(&self, request_id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM approval_requests WHERE id::text = $1")
            .bind(request_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("status of request {request_id}: {e}"))
    }

    /// Every decision row of one request: `(approver_id, approver_role, outcome, reason)`.
    pub async fn decisions(&self, request_id: &str) -> Vec<(String, String, String, String)> {
        sqlx::query_as(
            "SELECT approver_id, approver_role, outcome, reason \
             FROM approval_decisions WHERE request_id::text = $1",
        )
        .bind(request_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("decisions of request {request_id}: {e}"))
    }

    /// Every audit row whose actor is `actor_id`: `(actor_role, action)`.
    pub async fn audit_by(&self, actor_id: &str) -> Vec<(String, String)> {
        sqlx::query_as("SELECT actor_role, action FROM audit_events WHERE actor_id = $1")
            .bind(actor_id)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("audit rows of {actor_id}: {e}"))
    }

    /// Drop the schema. Reached only by a test that passed; a failed test
    /// leaves its schema behind for inspection.
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
