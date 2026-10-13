//! VF-6 (#1394) up-front tests: `VerificationSweeper` (expiry of `VERIFIED` cases and document purge).
//!
//! Locks design `docs/design/blueprint/verification.md` §9.1 VF-6, §9 edge cases, and ticket #1394:
//! - `VERIFIED` past `expires_at` by one second -> `EXPIRED`; not yet passed -> stays `VERIFIED` (V4);
//! - nothing ever moves to `VERIFIED` without a reviewer action (the sweeper run over every state);
//! - `REVOKED` and `REJECTED` cases are never touched by `expire`;
//! - purge deletes the object of a document past `retention_until` and keeps the row with `purged_at`;
//! - a `NULL` `retention_until` is never purged;
//! - ending a case sets each document's `retention_until` from its type's period (`VERIFICATION_PASSPORT`, `VERIFICATION_NATIONAL_ID`), `NULL` when the type has none;
//! - ending a case by reject or revoke sets each document's `retention_until` through `ReviewService` (ruling #1394);
//! - a document already `removed_at` is not purged twice; its row keeps `purged_at` once;
//! - store errors on one object do not stop the round;
//! - `DataType::from_str("VERIFICATION_PASSPORT")` and `"VERIFICATION_NATIONAL_ID"` round-trip through `as_str`.
//!
//! Written before the implementation by the test writer (gemini/studio-13, on the BE lane controller
//! claude/ad05bf8d's order); the implementer must not edit, delete or weaken them.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use futures::future::BoxFuture;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_customer::application::retention_job::RetentionJob;
use xc_customer::domain::retention::DataType;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_verification::application::review_service::ReviewService;
use xc_verification::application::sweeper::VerificationSweeper;
use xc_verification::domain::case::decision_reason::DecisionReason;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

const MUST_FINISH: StdDuration = StdDuration::from_secs(30);
static STARTING: Semaphore = Semaphore::const_new(1);

// ── In-memory DocumentStore with failure simulation ───────────────────────────

#[derive(Default)]
struct MemStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    deletes: AtomicUsize,
    fail_delete_ref: Mutex<Option<String>>,
}

impl MemStore {
    fn insert(&self, doc_ref: &str, bytes: &[u8]) {
        self.objects
            .lock()
            .expect("not poisoned")
            .insert(doc_ref.to_owned(), bytes.to_vec());
    }

    fn contains(&self, doc_ref: &str) -> bool {
        self.objects
            .lock()
            .expect("not poisoned")
            .contains_key(doc_ref)
    }

    fn fail_delete_on(&self, doc_ref: &str) {
        *self.fail_delete_ref.lock().expect("not poisoned") = Some(doc_ref.to_owned());
    }
}

impl DocumentStore for MemStore {
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, Result<String, DocumentStoreError>> {
        Box::pin(async move {
            self.objects
                .lock()
                .expect("not poisoned")
                .insert(doc_ref.to_owned(), bytes.to_vec());
            Ok("key-pii-1".to_owned())
        })
    }

    fn get<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<Vec<u8>, DocumentStoreError>> {
        Box::pin(async move {
            self.objects
                .lock()
                .expect("not poisoned")
                .get(doc_ref)
                .cloned()
                .ok_or_else(|| DocumentStoreError::NotFound(doc_ref.to_owned()))
        })
    }

    fn delete<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<(), DocumentStoreError>> {
        Box::pin(async move {
            if let Some(target) = self
                .fail_delete_ref
                .lock()
                .expect("not poisoned")
                .as_deref()
            {
                if target == doc_ref {
                    return Err(DocumentStoreError::Io("simulated disk error".to_owned()));
                }
            }
            self.deletes.fetch_add(1, Ordering::Relaxed);
            self.objects.lock().expect("not poisoned").remove(doc_ref);
            Ok(())
        })
    }
}

// ── Database setup ────────────────────────────────────────────────────────────

const ORG_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000001);
const BRANCH_ID: &str = "00000000-0000-0000-0000-000000000002";
const REVIEWER_USER_ID: Uuid = Uuid::from_u128(0x11111111_1111_1111_1111_111111111111);

fn unique(prefix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}_{}_{}_{count}",
        Utc::now().timestamp_micros(),
        std::process::id()
    )
}

struct Db {
    pool: PgPool,
    schema: String,
    store: Arc<MemStore>,
}

impl Db {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The verification tests need a real PostgreSQL; \
                 point it at an existing database whose name ends in `_test`"
            )
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("vf6_sw");
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
        {
            let _slot = STARTING.acquire().await.expect("never closed");
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
            Migrator::new(dir.as_path())
                .await
                .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
                .run(&pool)
                .await
                .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));

            let has_verification: bool =
                sqlx::query_scalar("SELECT to_regclass('verification_cases') IS NOT NULL")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(false);

            if !has_verification {
                sqlx::raw_sql(AssertSqlSafe(
                    r#"
CREATE TABLE IF NOT EXISTS verification_policies (
    organization_id         uuid        NOT NULL REFERENCES organizations(id),
    version                 integer     NOT NULL CHECK (version > 0),
    accepted_document_types text[]      NOT NULL CHECK (cardinality(accepted_document_types) > 0),
    documents_required      integer     NOT NULL CHECK (documents_required > 0),
    branch_review_allowed   boolean     NOT NULL,
    published_by            uuid        NULL REFERENCES users(id),
    published_at            timestamptz NOT NULL,
    created_at              timestamptz NOT NULL,
    PRIMARY KEY (organization_id, version),
    CHECK (version = 1 OR published_by IS NOT NULL)
);

CREATE TABLE IF NOT EXISTS verification_cases (
    id                   uuid        PRIMARY KEY,
    organization_id      uuid        NOT NULL REFERENCES organizations(id),
    customer_id          text        NOT NULL REFERENCES customers(id),
    provider             text        NOT NULL DEFAULT 'STAFF' CHECK (provider IN ('STAFF')),
    external_case_ref    text        NULL CHECK (external_case_ref IS NULL),
    status               text        NOT NULL CHECK (status IN ('DRAFT', 'SUBMITTED', 'UNDER_REVIEW', 'VERIFIED',
                                                                'REJECTED', 'NEEDS_UPDATE', 'EXPIRED', 'REVOKED')),
    policy_version       integer     NOT NULL,
    branch_id            text        NULL,
    submitted_at         timestamptz NULL,
    decided_at           timestamptz NULL,
    expires_at           timestamptz NULL,
    reviewed_by          uuid        NULL REFERENCES users(id),
    decision_reason_code text        NULL CHECK (decision_reason_code IN ('DOCUMENT_UNREADABLE', 'DOCUMENT_EXPIRED',
                                                                          'DETAILS_MISMATCH', 'DOCUMENT_NOT_ACCEPTED',
                                                                          'SUSPECTED_FRAUD', 'OTHER')),
    version              integer     NOT NULL DEFAULT 1,
    created_at           timestamptz NOT NULL,
    updated_at           timestamptz NULL,
    FOREIGN KEY (organization_id, policy_version) REFERENCES verification_policies (organization_id, version),
    FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id),
    CHECK (status NOT IN ('VERIFIED', 'REJECTED', 'NEEDS_UPDATE', 'REVOKED')
           OR (reviewed_by IS NOT NULL AND decided_at IS NOT NULL)),
    CHECK (status NOT IN ('REJECTED', 'NEEDS_UPDATE', 'REVOKED') OR decision_reason_code IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS verification_one_open_case ON verification_cases (customer_id)
    WHERE status IN ('DRAFT', 'SUBMITTED', 'UNDER_REVIEW', 'NEEDS_UPDATE');

CREATE TABLE IF NOT EXISTS verification_documents (
    id                   uuid        PRIMARY KEY,
    organization_id      uuid        NOT NULL REFERENCES organizations(id),
    case_id              uuid        NOT NULL REFERENCES verification_cases(id),
    document_type        text        NOT NULL,
    encrypted_object_ref text        NULL UNIQUE,
    key_id               text        NULL,
    checksum             bytea       NOT NULL,
    mime                 text        NOT NULL CHECK (mime IN ('image/jpeg', 'image/png', 'application/pdf')),
    size_bytes           integer     NOT NULL CHECK (size_bytes BETWEEN 1 AND 15728640),
    uploaded_at          timestamptz NOT NULL,
    uploaded_by          uuid        NOT NULL,
    retention_until      timestamptz NULL,
    access_class         text        NOT NULL DEFAULT 'IDENTITY_DOCUMENT' CHECK (access_class IN ('IDENTITY_DOCUMENT')),
    removed_at           timestamptz NULL,
    purged_at            timestamptz NULL,
    created_at           timestamptz NOT NULL,
    CHECK ((encrypted_object_ref IS NULL) = (removed_at IS NOT NULL OR purged_at IS NOT NULL))
);
"#,
                ))
                .execute(&pool)
                .await
                .expect("ensure verification tables");
            }
        }

        Self::seed(&pool).await;
        let store = Arc::new(MemStore::default());
        Self {
            pool,
            schema,
            store,
        }
    }

    async fn seed(pool: &PgPool) {
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, 'XC', 'Xchange Co.', 'Asia/Bangkok', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(ORG_ID)
        .execute(pool)
        .await
        .expect("seed org");

        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) \
             VALUES ($1, 'Head Office', $2) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(BRANCH_ID)
        .bind(ORG_ID)
        .execute(pool)
        .await
        .expect("seed branch");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2, 'Reviewer', 'ACTIVE', 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(REVIEWER_USER_ID)
        .bind(ORG_ID)
        .execute(pool)
        .await
        .expect("seed reviewer user");

        sqlx::query(
            "INSERT INTO verification_policies (organization_id, version, accepted_document_types, documents_required, branch_review_allowed, published_by, published_at, created_at) \
             VALUES ($1, 1, ARRAY['PASSPORT', 'NATIONAL_ID']::text[], 1, true, NULL, now(), now()) \
             ON CONFLICT (organization_id, version) DO NOTHING",
        )
        .bind(ORG_ID)
        .execute(pool)
        .await
        .expect("seed verification policy v1");
    }

    async fn create_customer(&self, customer_id: &str) {
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2, 'Customer', 'ACTIVE', 'test') \
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(ORG_ID)
        .execute(&self.pool)
        .await
        .expect("create customer user");

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ($1, $2, $1, 'Test Customer', 'LIVE', now(), $3) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(customer_id)
        .bind(ORG_ID)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .expect("create customer");
    }

    async fn set_retention_policy(&self, data_type: &str, period_days: Option<i32>) {
        sqlx::query(
            "INSERT INTO retention_policies (organization_id, data_type, period_days, set_by, set_at) \
             VALUES ($1, $2, $3, $4, now()) \
             ON CONFLICT (organization_id, data_type) \
             DO UPDATE SET period_days = EXCLUDED.period_days, set_at = now()",
        )
        .bind(ORG_ID)
        .bind(data_type)
        .bind(period_days)
        .bind(REVIEWER_USER_ID)
        .execute(&self.pool)
        .await
        .expect("set retention policy");
    }

    fn sweeper(&self) -> VerificationSweeper {
        VerificationSweeper::from_pool(self.pool.clone(), self.store.clone())
    }

    fn review_service(&self) -> ReviewService {
        ReviewService::new(
            self.pool.clone(),
            Arc::new(PgCaseRepository::new(self.pool.clone())),
            Arc::new(PgAuditLog::new(self.pool.clone())),
        )
    }

    async fn text(&self, query: &str) -> Option<String> {
        let row: Option<(Option<String>,)> = sqlx::query_as(AssertSqlSafe(query.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .expect("text query");
        row.and_then(|(s,)| s)
    }

    async fn count(&self, query: &str) -> i64 {
        let (n,): (i64,) = sqlx::query_as(AssertSqlSafe(query.to_owned()))
            .fetch_one(&self.pool)
            .await
            .expect("count query");
        n
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        let schema = self.schema.clone();
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let _ = sqlx::query(AssertSqlSafe(format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE"
            )))
            .execute(&pool)
            .await;
        });
    }
}

// ── 1. DataType round-trip and retentionSet acceptance ─────────────────────────

#[tokio::test]
async fn data_type_verification_variants_round_trip_and_retention_set_accepts_both() {
    let db = Db::start().await;

    // Both variants round-trip through as_str and from_str (Done criteria 2, V13)
    assert_eq!(
        DataType::from_str("VERIFICATION_PASSPORT").map(|d| d.as_str()),
        Ok("VERIFICATION_PASSPORT")
    );
    assert_eq!(
        DataType::from_str("VERIFICATION_NATIONAL_ID").map(|d| d.as_str()),
        Ok("VERIFICATION_NATIONAL_ID")
    );

    let passport = DataType::from_str("VERIFICATION_PASSPORT").expect("parse passport");
    let national_id = DataType::from_str("VERIFICATION_NATIONAL_ID").expect("parse national id");

    let org_str = ORG_ID.to_string();
    let reviewer_str = REVIEWER_USER_ID.to_string();

    // Existing and new data types are accepted by retentionSet (RetentionJob)
    RetentionJob::pg(db.pool.clone())
        .set_period(
            &org_str,
            Some(&reviewer_str),
            passport,
            Some(30),
            Utc::now(),
        )
        .await
        .expect("retention set passport");

    RetentionJob::pg(db.pool.clone())
        .set_period(
            &org_str,
            Some(&reviewer_str),
            national_id,
            Some(60),
            Utc::now(),
        )
        .await
        .expect("retention set national id");

    let period_passport: Option<i32> = sqlx::query_scalar(
        "SELECT period_days FROM retention_policies WHERE organization_id = $1 AND data_type = $2",
    )
    .bind(ORG_ID)
    .bind("VERIFICATION_PASSPORT")
    .fetch_one(&db.pool)
    .await
    .expect("fetch passport retention");
    assert_eq!(period_passport, Some(30));

    let period_id: Option<i32> = sqlx::query_scalar(
        "SELECT period_days FROM retention_policies WHERE organization_id = $1 AND data_type = $2",
    )
    .bind(ORG_ID)
    .bind("VERIFICATION_NATIONAL_ID")
    .fetch_one(&db.pool)
    .await
    .expect("fetch national id retention");
    assert_eq!(period_id, Some(60));
}

// ── 2. Expiry: 1s past expires_at moves to EXPIRED, not yet passed stays VERIFIED ─

#[tokio::test]
async fn verified_case_past_expires_at_by_one_second_moves_to_expired() {
    let db = Db::start().await;
    let cust_id = unique("cust_v4_exp");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    let expiry_time = Utc::now() + Duration::hours(2);

    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'VERIFIED', 1, now(), now(), $4, $5, 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(expiry_time)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert verified case");

    let sweeper = db.sweeper();

    // 1 second before expires_at -> stays VERIFIED (V4 "once expires_at passes")
    let before = expiry_time - Duration::seconds(1);
    let moved_before = sweeper
        .expire(before)
        .await
        .expect("expire before should succeed");
    assert_eq!(moved_before, 0, "not yet passed expires_at -> 0 moved");
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await
        .as_deref(),
        Some("VERIFIED"),
        "stays VERIFIED before expires_at"
    );

    // 1 second after expires_at -> moves to EXPIRED
    let after = expiry_time + Duration::seconds(1);
    let moved_after = sweeper
        .expire(after)
        .await
        .expect("expire after should succeed");
    assert_eq!(
        moved_after, 1,
        "past expires_at by 1s -> 1 moved to EXPIRED"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await
        .as_deref(),
        Some("EXPIRED"),
        "moved to EXPIRED"
    );
}

// ── 3. Sweeper run over every state: nothing ever moves to VERIFIED ────────────

#[tokio::test]
async fn nothing_ever_moves_to_verified_without_reviewer_action_sweeper_run_over_every_state() {
    let db = Db::start().await;
    let non_verified_states = [
        "DRAFT",
        "SUBMITTED",
        "UNDER_REVIEW",
        "NEEDS_UPDATE",
        "REJECTED",
        "EXPIRED",
        "REVOKED",
    ];

    let past_time = Utc::now() - Duration::days(1);
    let mut case_ids = Vec::new();

    for (idx, status) in non_verified_states.iter().enumerate() {
        let cust_id = unique(&format!("cust_state_{idx}"));
        db.create_customer(&cust_id).await;
        let case_id = Uuid::new_v4();
        case_ids.push((case_id, *status));

        let reviewed_by = match *status {
            "REJECTED" | "NEEDS_UPDATE" | "REVOKED" => Some(REVIEWER_USER_ID),
            _ => None,
        };
        let reason = match *status {
            "REJECTED" | "NEEDS_UPDATE" | "REVOKED" => Some("DOCUMENT_EXPIRED"),
            _ => None,
        };

        sqlx::query(
            "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
             submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at) \
             VALUES ($1, $2, $3, 'STAFF', $4, 1, now(), now(), $5, $6, $7, 1, now())",
        )
        .bind(case_id)
        .bind(ORG_ID)
        .bind(&cust_id)
        .bind(status)
        .bind(past_time)
        .bind(reviewed_by)
        .bind(reason)
        .execute(&db.pool)
        .await
        .expect("insert case");
    }

    let sweeper = db.sweeper();
    let now = Utc::now();

    // Run sweeper expire and purge
    let _ = sweeper.expire(now).await;
    let _ = sweeper.purge(now).await;

    // Invariant: nothing ever moves to VERIFIED without a reviewer action (design §9.1 VF-6)
    for (case_id, original_status) in case_ids {
        let current_status = db
            .text(&format!(
                "SELECT status FROM verification_cases WHERE id = '{case_id}'"
            ))
            .await
            .expect("fetch current status");

        assert_ne!(
            current_status, "VERIFIED",
            "case {case_id} (orig {original_status}) must never move to VERIFIED"
        );
    }
}

// ── 4. REVOKED / REJECTED cases are never touched by expire ───────────────────

#[tokio::test]
async fn revoked_and_rejected_cases_are_never_touched_by_expire() {
    let db = Db::start().await;
    let cust_rev = unique("cust_rev");
    let cust_rej = unique("cust_rej");
    db.create_customer(&cust_rev).await;
    db.create_customer(&cust_rej).await;

    let case_rev = Uuid::new_v4();
    let case_rej = Uuid::new_v4();
    let past = Utc::now() - Duration::hours(5);

    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'REVOKED', 1, now(), $4, $5, 'SUSPECTED_FRAUD', 2, now())",
    )
    .bind(case_rev)
    .bind(ORG_ID)
    .bind(&cust_rev)
    .bind(past)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert revoked case");

    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'REJECTED', 1, now(), $4, $5, 'DOCUMENT_UNREADABLE', 2, now())",
    )
    .bind(case_rej)
    .bind(ORG_ID)
    .bind(&cust_rej)
    .bind(past)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert rejected case");

    let sweeper = db.sweeper();
    let moved = sweeper.expire(Utc::now()).await.expect("expire run");
    assert_eq!(moved, 0, "REVOKED and REJECTED are never expired");

    assert_eq!(
        db.text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_rev}'"
        ))
        .await
        .as_deref(),
        Some("REVOKED")
    );
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_rej}'"
        ))
        .await
        .as_deref(),
        Some("REJECTED")
    );
}

// ── 5. Ending a case sets document retention_until from type period ────────────

#[tokio::test]
async fn ending_a_case_sets_each_documents_retention_until_from_its_types_period() {
    let db = Db::start().await;
    let cust_id = unique("cust_ret_end");
    db.create_customer(&cust_id).await;

    // Organization sets retention periods: PASSPORT = 30 days, NATIONAL_ID = 60 days
    db.set_retention_policy("VERIFICATION_PASSPORT", Some(30))
        .await;
    db.set_retention_policy("VERIFICATION_NATIONAL_ID", Some(60))
        .await;

    let case_id = Uuid::new_v4();
    let expiry = Utc::now() - Duration::minutes(10);

    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'VERIFIED', 1, now(), $4, $5, 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(expiry)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert verified case");

    let doc_passport = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', 'ref-p1', '\\x1234', 'image/jpeg', 1024, now(), $4, NULL, now()), \
                ($5, $2, $3, 'NATIONAL_ID', 'ref-n1', '\\x5678', 'image/png', 2048, now(), $4, NULL, now())",
    )
    .bind(doc_passport)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(REVIEWER_USER_ID)
    .bind(doc_id)
    .execute(&db.pool)
    .await
    .expect("insert documents");

    let sweeper = db.sweeper();
    let now = Utc::now();
    let moved = sweeper.expire(now).await.expect("expire case");
    assert_eq!(moved, 1, "case expired");

    // Document retention_until is set to ended_at (now) + period_days
    let p_ret: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_passport)
            .fetch_one(&db.pool)
            .await
            .expect("fetch passport retention");

    let n_ret: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch national id retention");

    assert!(p_ret.is_some(), "passport retention_until must be set");
    assert!(n_ret.is_some(), "national id retention_until must be set");

    let p_expected = now + Duration::days(30);
    let n_expected = now + Duration::days(60);

    let p_diff = (p_ret.unwrap() - p_expected).num_seconds().abs();
    let n_diff = (n_ret.unwrap() - n_expected).num_seconds().abs();

    assert!(
        p_diff < 5,
        "passport retention ~30 days from expiry: {p_diff}s diff"
    );
    assert!(
        n_diff < 5,
        "national id retention ~60 days from expiry: {n_diff}s diff"
    );
}

// ── 6. Ending a case with no retention period leaves retention_until NULL ─────

#[tokio::test]
async fn ending_a_case_with_no_retention_period_leaves_retention_until_null() {
    let db = Db::start().await;
    let cust_id = unique("cust_ret_null");
    db.create_customer(&cust_id).await;

    // No retention period configured for VERIFICATION_PASSPORT (NULL period)
    db.set_retention_policy("VERIFICATION_PASSPORT", None).await;

    let case_id = Uuid::new_v4();
    let expiry = Utc::now() - Duration::minutes(5);

    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'VERIFIED', 1, now(), $4, $5, 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(expiry)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert case");

    let doc_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', 'ref-p2', '\\x1234', 'image/jpeg', 1024, now(), $4, NULL, now())",
    )
    .bind(doc_id)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert document");

    let sweeper = db.sweeper();
    let _ = sweeper.expire(Utc::now()).await.expect("expire case");

    let ret: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch retention");

    assert!(
        ret.is_none(),
        "NULL retention_until when type has no period (U2)"
    );
}

// ── 7. Rejecting a case sets each document's retention_until from its type's period or NULL ─

#[tokio::test]
async fn rejecting_a_case_sets_each_documents_retention_until_from_its_types_period_or_null() {
    let db = Db::start().await;
    let cust_id = unique("cust_ret_rej");
    db.create_customer(&cust_id).await;

    // Organization sets retention periods: PASSPORT = 30 days, NATIONAL_ID has no period (NULL)
    db.set_retention_policy("VERIFICATION_PASSPORT", Some(30))
        .await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'UNDER_REVIEW', 1, now(), 1, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .execute(&db.pool)
    .await
    .expect("insert under_review case");

    let doc_passport = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', 'ref-p1', '\\x1234', 'image/jpeg', 1024, now(), $4, NULL, now()), \
                ($5, $2, $3, 'NATIONAL_ID', 'ref-n1', '\\x5678', 'image/png', 2048, now(), $4, NULL, now())",
    )
    .bind(doc_passport)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(REVIEWER_USER_ID)
    .bind(doc_id)
    .execute(&db.pool)
    .await
    .expect("insert documents");

    let reviewer = ReviewerActor {
        user_id: REVIEWER_USER_ID,
        scope: ReviewerScope::Organization {
            organization_id: ORG_ID,
        },
    };

    let service = db.review_service();
    let res = service
        .reject(&reviewer, case_id, 1, DecisionReason::DetailsMismatch)
        .await;
    assert!(res.is_ok(), "reject case must succeed: {:?}", res.err());

    let p_ret: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_passport)
            .fetch_one(&db.pool)
            .await
            .expect("fetch passport retention");

    let n_ret: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch national id retention");

    assert!(p_ret.is_some(), "passport retention_until must be set");
    assert!(
        n_ret.is_none(),
        "national id retention_until must be NULL when no period configured"
    );

    let now = Utc::now();
    let p_expected = now + Duration::days(30);
    let p_diff = (p_ret.unwrap() - p_expected).num_seconds().abs();
    assert!(
        p_diff < 5,
        "passport retention ~30 days from rejection: {p_diff}s diff"
    );
}

// ── 8. Revoking a case sets each document's retention_until from its type's period or NULL ─

#[tokio::test]
async fn revoking_a_case_sets_each_documents_retention_until_from_its_types_period_or_null() {
    let db = Db::start().await;
    let cust_id = unique("cust_ret_rev");
    db.create_customer(&cust_id).await;

    // Organization sets retention periods: PASSPORT = 30 days, NATIONAL_ID has no period (NULL)
    db.set_retention_policy("VERIFICATION_PASSPORT", Some(30))
        .await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'VERIFIED', 1, now(), now(), now() + interval '90 days', $4, 1, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert verified case");

    let doc_passport = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', 'ref-p1', '\\x1234', 'image/jpeg', 1024, now(), $4, NULL, now()), \
                ($5, $2, $3, 'NATIONAL_ID', 'ref-n1', '\\x5678', 'image/png', 2048, now(), $4, NULL, now())",
    )
    .bind(doc_passport)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(REVIEWER_USER_ID)
    .bind(doc_id)
    .execute(&db.pool)
    .await
    .expect("insert documents");

    let reviewer = ReviewerActor {
        user_id: REVIEWER_USER_ID,
        scope: ReviewerScope::Organization {
            organization_id: ORG_ID,
        },
    };

    let service = db.review_service();
    let res = service
        .revoke(&reviewer, case_id, 1, DecisionReason::SuspectedFraud)
        .await;
    assert!(res.is_ok(), "revoke case must succeed: {:?}", res.err());

    let p_ret: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_passport)
            .fetch_one(&db.pool)
            .await
            .expect("fetch passport retention");

    let n_ret: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch national id retention");

    assert!(p_ret.is_some(), "passport retention_until must be set");
    assert!(
        n_ret.is_none(),
        "national id retention_until must be NULL when no period configured"
    );

    let now = Utc::now();
    let p_expected = now + Duration::days(30);
    let p_diff = (p_ret.unwrap() - p_expected).num_seconds().abs();
    assert!(
        p_diff < 5,
        "passport retention ~30 days from revocation: {p_diff}s diff"
    );
}

// ── 9. Purge deletes object past retention_until and keeps row with purged_at ──

#[tokio::test]
async fn purge_deletes_object_past_retention_until_and_keeps_row_with_purged_at() {
    let db = Db::start().await;
    let cust_id = unique("cust_purge");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'REJECTED', 1, now(), $4, 'DOCUMENT_EXPIRED', 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert rejected case");

    let doc_id = Uuid::new_v4();
    let doc_ref = format!("doc-ref-purge-{}", Uuid::new_v4());
    db.store.insert(&doc_ref, b"secret-passport-image-bytes");

    let retention_target = Utc::now() + Duration::hours(5);

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', $4, '\\x1234', 'image/jpeg', 26, now(), $5, $6, now())",
    )
    .bind(doc_id)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(&doc_ref)
    .bind(REVIEWER_USER_ID)
    .bind(retention_target)
    .execute(&db.pool)
    .await
    .expect("insert document");

    let sweeper = db.sweeper();

    // 1 second before retention_until: object not purged
    let before = retention_target - Duration::seconds(1);
    let purged_before = sweeper.purge(before).await.expect("purge before");
    assert_eq!(purged_before, 0);
    assert!(db.store.contains(&doc_ref), "object still in store");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE id = '{doc_id}' AND purged_at IS NULL"
        ))
        .await,
        1
    );

    // 1 second after retention_until: object is purged
    let after = retention_target + Duration::seconds(1);
    let purged_after = sweeper.purge(after).await.expect("purge after");
    assert_eq!(purged_after, 1, "one object deleted from store");

    assert!(!db.store.contains(&doc_ref), "object deleted from store");
    assert_eq!(
        db.text(&format!(
            "SELECT encrypted_object_ref FROM verification_documents WHERE id = '{doc_id}'"
        ))
        .await,
        None,
        "encrypted_object_ref cleared"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE id = '{doc_id}' AND purged_at IS NOT NULL"
        ))
        .await,
        1,
        "purged_at recorded on the kept row"
    );
}

// ── 10. NULL retention_until is never purged ───────────────────────────────────

#[tokio::test]
async fn null_retention_until_is_never_purged() {
    let db = Db::start().await;
    let cust_id = unique("cust_no_purge");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'REJECTED', 1, now(), $4, 'OTHER', 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert rejected case");

    let doc_id = Uuid::new_v4();
    let doc_ref = format!("doc-ref-null-{}", Uuid::new_v4());
    db.store.insert(&doc_ref, b"permanent-evidence");

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', $4, '\\x1234', 'image/jpeg', 18, now(), $5, NULL, now())",
    )
    .bind(doc_id)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(&doc_ref)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert document with NULL retention_until");

    let sweeper = db.sweeper();
    // Run purge 10 years in the future
    let future = Utc::now() + Duration::days(3650);
    let purged = sweeper.purge(future).await.expect("purge run");
    assert_eq!(purged, 0, "NULL retention_until is never purged (U2)");

    assert!(db.store.contains(&doc_ref), "object kept in store");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE id = '{doc_id}' AND purged_at IS NULL"
        ))
        .await,
        1
    );
}

// ── 11. Document already removed_at is not purged twice ─────────────────────────

#[tokio::test]
async fn document_already_removed_is_not_purged_twice() {
    let db = Db::start().await;
    let cust_id = unique("cust_removed");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'DRAFT', 1, 1, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .execute(&db.pool)
    .await
    .expect("insert draft case");

    let doc_id = Uuid::new_v4();
    let past = Utc::now() - Duration::hours(1);

    // Document was already removed: encrypted_object_ref is NULL, removed_at is set
    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, removed_at, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', NULL, '\\x1234', 'image/jpeg', 10, now(), $4, $5, $5, now())",
    )
    .bind(doc_id)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(REVIEWER_USER_ID)
    .bind(past)
    .execute(&db.pool)
    .await
    .expect("insert removed document");

    let sweeper = db.sweeper();
    let purged = sweeper.purge(Utc::now()).await.expect("purge");
    assert_eq!(purged, 0, "already removed document is not purged twice");

    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE id = '{doc_id}' AND purged_at IS NULL"
        ))
        .await,
        1,
        "purged_at remains NULL"
    );
}

// ── 12. Store errors on one object do not stop the round ───────────────────────

#[tokio::test]
async fn store_error_on_one_object_does_not_stop_the_round() {
    let db = Db::start().await;
    let cust_id = unique("cust_err_round");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'REJECTED', 1, now(), $4, 'OTHER', 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert rejected case");

    let doc_bad = Uuid::new_v4();
    let doc_good = Uuid::new_v4();
    let ref_bad = format!("bad-ref-{}", Uuid::new_v4());
    let ref_good = format!("good-ref-{}", Uuid::new_v4());

    db.store.insert(&ref_bad, b"bad-object");
    db.store.insert(&ref_good, b"good-object");
    db.store.fail_delete_on(&ref_bad);

    let past = Utc::now() - Duration::hours(1);

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $2, $3, 'PASSPORT', $4, '\\x1111', 'image/jpeg', 10, now(), $5, $6, now()), \
                ($7, $2, $3, 'PASSPORT', $8, '\\x2222', 'image/jpeg', 10, now(), $5, $6, now())",
    )
    .bind(doc_bad)
    .bind(ORG_ID)
    .bind(case_id)
    .bind(&ref_bad)
    .bind(REVIEWER_USER_ID)
    .bind(past)
    .bind(doc_good)
    .bind(&ref_good)
    .execute(&db.pool)
    .await
    .expect("insert two documents");

    let sweeper = db.sweeper();
    // Sweeper purge should not abort when one object delete fails (design §8 [proposed])
    let purged = sweeper
        .purge(Utc::now())
        .await
        .expect("purge should not crash");
    assert_eq!(
        purged, 1,
        "good object was purged despite error on bad object"
    );

    assert!(!db.store.contains(&ref_good), "good object deleted");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE id = '{doc_good}' AND purged_at IS NOT NULL"
        ))
        .await,
        1
    );
}
