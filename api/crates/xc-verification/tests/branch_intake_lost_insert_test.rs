//! Ticket #2658 — Branch verification intake answers 400 caseLocked when its new case loses the insert race.
//!
//! Behaviour table:
//! - I1: lookup sees no open case (wrapper), but customer has open DRAFT case, insert hits ON CONFLICT DO NOTHING -> Err(CaseLocked), no verification_documents row for derived case, seeded case unchanged (main: Err(Case(Store(_))))
//! - I2: customer with no case at all (plain PgCaseRepository) -> new SUBMITTED case with the one document (keep)
//! - I3: customer with open DRAFT case that lookup sees (plain PgCaseRepository) -> document added to that case and submitted (keep)

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;
use xc_kernel::pii::identity_type::IdentityType;
use xc_verification::application::branch_intake::{
    BranchDocument, BranchIntake, BranchIntakeError,
};
use xc_verification::domain::case::case_error::CaseError;
use xc_verification::domain::case::case_status::CaseStatus;
use xc_verification::domain::case::provider::Provider;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::domain::case::verification_case::VerificationCase;
use xc_verification::domain::case_repository::CaseRepository;
use xc_verification::domain::document::verification_document::VerificationDocument;
use xc_verification::domain::policy::verification_policy::VerificationPolicy;
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const STORE_KEY_ID: &str = "test-vault-key-1";
const MUST_FINISH: Duration = Duration::from_secs(60);

static NEXT: AtomicU64 = AtomicU64::new(0);
static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn png() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(b"VF12-SECRET-BYTES-9d4f");
    b.extend_from_slice(&unique("png").into_bytes());
    b
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

#[derive(Default)]
struct MemStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    puts: AtomicU64,
}

impl DocumentStore for MemStore {
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, Result<String, DocumentStoreError>> {
        self.puts.fetch_add(1, Ordering::Relaxed);
        let mut g = self.objects.lock().unwrap();
        g.insert(doc_ref.to_owned(), bytes.to_vec());
        Box::pin(async move { Ok(STORE_KEY_ID.to_owned()) })
    }

    fn get<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<Vec<u8>, DocumentStoreError>> {
        let g = self.objects.lock().unwrap();
        let res = g
            .get(doc_ref)
            .cloned()
            .ok_or_else(|| DocumentStoreError::NotFound(doc_ref.to_owned()));
        Box::pin(async move { res })
    }

    fn delete<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<(), DocumentStoreError>> {
        let mut g = self.objects.lock().unwrap();
        g.remove(doc_ref);
        Box::pin(async move { Ok(()) })
    }
}

/// Delegating wrapper for `CaseRepository` that returns `Ok(None)` for
/// `find_open_case_by_customer`, simulating a race where lookup found no open case
/// but another case was inserted before this transaction's insert.
struct LostInsertCaseRepository {
    inner: Arc<PgCaseRepository>,
}

impl CaseRepository for LostInsertCaseRepository {
    fn find_customer<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<(Uuid, Option<Uuid>)>, CaseError>> {
        self.inner.find_customer(tx, customer_id)
    }

    fn find_latest_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationPolicy>, CaseError>> {
        self.inner.find_latest_policy(tx, organization_id)
    }

    fn find_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        version: i32,
    ) -> BoxFuture<'a, Result<Option<VerificationPolicy>, CaseError>> {
        self.inner.find_policy(tx, organization_id, version)
    }

    fn find_open_case_by_customer<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>> {
        Box::pin(async move { Ok(None) })
    }

    fn insert_case<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case: &'a VerificationCase,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>> {
        self.inner.insert_case(tx, case)
    }

    fn find_case_by_id_for_update<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>> {
        self.inner.find_case_by_id_for_update(tx, case_id)
    }

    fn update_case_submitted<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
        submitted_at: DateTime<Utc>,
        version: i32,
    ) -> BoxFuture<'a, Result<(), CaseError>> {
        self.inner
            .update_case_submitted(tx, case_id, submitted_at, version)
    }

    fn update_case_submitted_with_branch<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
        submitted_at: DateTime<Utc>,
        version: i32,
        branch_id: Option<&'a BranchId>,
    ) -> BoxFuture<'a, Result<(), CaseError>> {
        self.inner
            .update_case_submitted_with_branch(tx, case_id, submitted_at, version, branch_id)
    }

    fn find_document_by_ref<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_ref: &'a str,
    ) -> BoxFuture<'a, Result<Option<VerificationDocument>, CaseError>> {
        self.inner.find_document_by_ref(tx, object_ref)
    }

    fn find_document_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationDocument>, CaseError>> {
        self.inner.find_document_by_id(tx, id)
    }

    fn list_active_documents_by_case<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
    ) -> BoxFuture<'a, Result<Vec<VerificationDocument>, CaseError>> {
        self.inner.list_active_documents_by_case(tx, case_id)
    }

    fn insert_document<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        doc: &'a VerificationDocument,
    ) -> BoxFuture<'a, Result<(), CaseError>> {
        self.inner.insert_document(tx, organization_id, doc)
    }

    fn remove_document<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), CaseError>> {
        self.inner.remove_document(tx, id, now)
    }
}

struct Db {
    pool: PgPool,
    schema: String,
    store: Arc<MemStore>,
}

#[allow(dead_code)]
struct Customer {
    id: String,
    user: Uuid,
    organization_id: String,
}

impl Db {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. Point it at an existing database whose name ends in `_test`"
            )
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("lost_ins");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
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

        {
            let _slot = STARTING.acquire().await.expect("slot");
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
            Migrator::new(dir.as_path())
                .await
                .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
                .run(&pool)
                .await
                .unwrap_or_else(|e| panic!("run migrations: {e}"));
        }

        Self {
            pool,
            schema,
            store: Arc::new(MemStore::default()),
        }
    }

    async fn finish(self) {
        let Db { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }

    fn intake(&self) -> BranchIntake {
        BranchIntake::new(
            self.pool.clone(),
            Arc::new(PgCaseRepository::new(self.pool.clone())),
            self.store.clone(),
            Arc::new(PgAuditLog::new(self.pool.clone())),
        )
    }

    fn intake_with_repo(&self, repo: Arc<dyn CaseRepository>) -> BranchIntake {
        BranchIntake::new(
            self.pool.clone(),
            repo,
            self.store.clone(),
            Arc::new(PgAuditLog::new(self.pool.clone())),
        )
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        let (n,): (i64,) = sqlx::query_as(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        n
    }

    async fn cases_count(&self) -> i64 {
        self.count("SELECT count(*) FROM verification_cases").await
    }

    async fn documents_count(&self) -> i64 {
        self.count("SELECT count(*) FROM verification_documents")
            .await
    }

    async fn user(&self, org: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{id}', '{org}'::uuid, '{}', 'ACTIVE', 'test')",
            unique("user")
        ))
        .await;
        id
    }

    async fn customer(&self, org: &str) -> Customer {
        let user = self.user(org).await;
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'Somchai', 'LIVE', now(), '{user}')"
        ))
        .await;
        Customer {
            id,
            user,
            organization_id: org.to_string(),
        }
    }

    async fn branch(&self, org: &str) -> BranchId {
        let id = unique("B").replace('_', "-");
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{id}', '{id}', '{org}'::uuid)"
        ))
        .await;
        BranchId::new(id)
    }

    async fn branch_reviewer(&self, org: &str) -> (ReviewerActor, BranchId) {
        let user_id = self.user(org).await;
        let branch_id = self.branch(org).await;
        (
            ReviewerActor {
                user_id,
                scope: ReviewerScope::Branch {
                    organization_id: Uuid::parse_str(org).expect("uuid"),
                    branch_id: branch_id.clone(),
                },
            },
            branch_id,
        )
    }

    async fn seed_customer_identity(&self, customer_id: &str, org: &str) {
        let secret_ident = "PASSPORT-IDENT-123456789";
        self.exec(&format!(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, nationality, status) \
             VALUES (gen_random_uuid(), '{org}'::uuid, '{customer_id}', 'PASSPORT', '{secret_ident}'::bytea, 'k-ident', gen_random_uuid()::text::bytea, 'THA', 'ACTIVE')"
        ))
        .await;
    }

    async fn force_open_case(&self, customer_id: &str, org: &str, status: &str) -> Uuid {
        let id = Uuid::new_v4();
        let decided = !matches!(status, "DRAFT" | "SUBMITTED" | "UNDER_REVIEW");
        let reasoned = matches!(status, "REJECTED" | "NEEDS_UPDATE" | "REVOKED");
        let (reviewed_by, decided_at) = if decided {
            let user = self.user(org).await;
            (format!("'{user}'::uuid"), "now()")
        } else {
            ("NULL".to_owned(), "NULL")
        };
        let reason = if reasoned { "'OTHER'" } else { "NULL" };
        let submitted = if status == "DRAFT" { "NULL" } else { "now()" };
        self.exec(&format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, \
             policy_version, version, submitted_at, decided_at, reviewed_by, decision_reason_code, created_at) \
             VALUES ('{id}', '{org}'::uuid, '{customer_id}', 'STAFF', '{status}', 1, 1, {submitted}, {decided_at}, {reviewed_by}, {reason}, now())"
        ))
        .await;
        id
    }
}

// ── Behaviour table tests ───────────────────────────────────────────────────

#[tokio::test]
async fn i1_lost_insert_answers_case_locked() {
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    db.seed_customer_identity(&customer.id, ORG_1).await;

    // Seed customer with an open DRAFT case
    let seeded_case_id = db.force_open_case(&customer.id, ORG_1, "DRAFT").await;

    let lost_repo = Arc::new(LostInsertCaseRepository {
        inner: Arc::new(PgCaseRepository::new(db.pool.clone())),
    });
    let intake = db.intake_with_repo(lost_repo);

    let bytes = png();
    let doc = BranchDocument::new(IdentityType::Passport, bytes, "k-race-1");

    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("lost insert must return Err(CaseLocked)");

    assert!(
        matches!(err, BranchIntakeError::CaseLocked),
        "expected BranchIntakeError::CaseLocked, got: {err:?}"
    );

    // No document row written
    assert_eq!(
        db.documents_count().await,
        0,
        "no verification_documents row may be written"
    );

    // The seeded case is unchanged
    assert_eq!(
        db.cases_count().await,
        1,
        "exactly the 1 seeded case must exist"
    );
    let (status, version): (String, i32) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT status, version FROM verification_cases WHERE id = '{seeded_case_id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("fetch seeded case");

    assert_eq!(status, "DRAFT", "seeded case status must remain DRAFT");
    assert_eq!(version, 1, "seeded case version must remain 1");

    db.finish().await;
}

#[tokio::test]
async fn i2_new_case_without_existing_case() {
    let db = Db::start().await;
    let (reviewer, branch_id) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    db.seed_customer_identity(&customer.id, ORG_1).await;

    let intake = db.intake();
    let bytes = png();
    let expected_hash = sha256(&bytes);
    let doc = BranchDocument::new(IdentityType::Passport, bytes.clone(), "k-new-1");

    let (case, document) = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect("intake succeeds for new customer");

    assert_eq!(case.status, CaseStatus::Submitted);
    assert_eq!(case.branch_id, Some(branch_id));
    assert_eq!(case.provider, Provider::Staff);
    assert_eq!(case.customer_id, customer.id);
    assert_eq!(case.policy_version, 1);
    assert_eq!(case.version, 1);

    assert_eq!(document.case_id, case.id);
    assert_eq!(document.document_type, IdentityType::Passport);
    assert_eq!(document.checksum, expected_hash);

    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 1);

    db.finish().await;
}

#[tokio::test]
async fn i3_existing_draft_case_found_and_submitted() {
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    db.seed_customer_identity(&customer.id, ORG_1).await;

    let draft_case_id = db.force_open_case(&customer.id, ORG_1, "DRAFT").await;

    let intake = db.intake();
    let bytes = png();
    let doc = BranchDocument::new(IdentityType::Passport, bytes, "k-existing-1");

    let (case, document) = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect("intake succeeds for existing draft case");

    assert_eq!(case.id, draft_case_id);
    assert_eq!(case.status, CaseStatus::Submitted);
    assert_eq!(document.case_id, draft_case_id);

    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 1);

    db.finish().await;
}
