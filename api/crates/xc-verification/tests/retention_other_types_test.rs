//! #1936 (§9 row 3): ending a verification case sets `retention_until` for `PASSPORT` and
//! `NATIONAL_ID` documents from their type's retention policy while leaving any other
//! `document_type` untouched.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, SubsecRound, Utc};
use futures::future::BoxFuture;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_verification::application::review_service::ReviewService;
use xc_verification::application::sweeper::VerificationSweeper;
use xc_verification::domain::case::decision_reason::DecisionReason;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

const MUST_FINISH: StdDuration = StdDuration::from_secs(30);
const ORG_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000001);
const BRANCH_ID: &str = "00000000-0000-0000-0000-000000000002";
const REVIEWER_USER_ID: Uuid = Uuid::from_u128(0x11111111_1111_1111_1111_111111111111);

#[derive(Default)]
struct MemStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
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
            self.objects.lock().expect("not poisoned").remove(doc_ref);
            Ok(())
        })
    }
}

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
        let schema = unique("vf_ret_other");
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
            .max_connections(4)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));

        Self::seed(&pool).await;
        Self {
            pool,
            schema,
            store: Arc::new(MemStore::default()),
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

    async fn retention_until(&self, doc_id: Uuid) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT retention_until FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&self.pool)
            .await
            .expect("fetch document retention_until")
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

#[tokio::test]
async fn ending_a_case_leaves_other_document_types_untouched_while_setting_passport_and_national_id()
 {
    let db = Db::start().await;

    // PASSPORT has a 30-day period; NATIONAL_ID has no period (NULL).
    db.set_retention_policy("VERIFICATION_PASSPORT", Some(30))
        .await;
    db.set_retention_policy("VERIFICATION_NATIONAL_ID", None)
        .await;

    let known_other_retention = (Utc::now() + Duration::days(365)).trunc_subsecs(6);

    // 1. Reject path through ReviewService::reject
    let cust_rej = unique("cust_other_rej");
    db.create_customer(&cust_rej).await;
    let case_rej = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'UNDER_REVIEW', 1, now(), 1, now())",
    )
    .bind(case_rej)
    .bind(ORG_ID)
    .bind(&cust_rej)
    .execute(&db.pool)
    .await
    .expect("insert under_review case");

    let doc_passport_rej = Uuid::new_v4();
    let doc_nat_id_rej = Uuid::new_v4();
    let doc_other_rej = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $4, $5, 'PASSPORT', 'ref-p-rej', '\\x1234', 'image/jpeg', 1024, now(), $6, NULL, now()), \
                ($2, $4, $5, 'NATIONAL_ID', 'ref-n-rej', '\\x5678', 'image/png', 2048, now(), $6, $7, now()), \
                ($3, $4, $5, 'OTHER', 'ref-o-rej', '\\x9abc', 'application/pdf', 512, now(), $6, $7, now())",
    )
    .bind(doc_passport_rej)
    .bind(doc_nat_id_rej)
    .bind(doc_other_rej)
    .bind(ORG_ID)
    .bind(case_rej)
    .bind(REVIEWER_USER_ID)
    .bind(known_other_retention)
    .execute(&db.pool)
    .await
    .expect("insert documents for reject case");

    let reviewer = ReviewerActor {
        user_id: REVIEWER_USER_ID,
        scope: ReviewerScope::Organization {
            organization_id: ORG_ID,
        },
    };
    db.review_service()
        .reject(&reviewer, case_rej, 1, DecisionReason::DetailsMismatch)
        .await
        .expect("reject case");

    let p_ret_rej = db
        .retention_until(doc_passport_rej)
        .await
        .expect("PASSPORT retention_until must be set on reject");
    assert!(
        (p_ret_rej - (Utc::now() + Duration::days(30)))
            .num_seconds()
            .abs()
            < 5,
        "PASSPORT retention_until must be ~30 days from reject, got {p_ret_rej}"
    );
    assert_eq!(
        db.retention_until(doc_nat_id_rej).await,
        None,
        "NATIONAL_ID retention_until must be set to NULL when its policy period is None"
    );
    assert_eq!(
        db.retention_until(doc_other_rej).await,
        Some(known_other_retention),
        "OTHER document_type retention_until must remain untouched on reject"
    );

    // 2. Expiry path through VerificationSweeper::expire
    let cust_exp = unique("cust_other_exp");
    db.create_customer(&cust_exp).await;
    let case_exp = Uuid::new_v4();
    let expired_at = Utc::now() - Duration::minutes(5);
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'VERIFIED', 1, now(), $4, $5, 2, now())",
    )
    .bind(case_exp)
    .bind(ORG_ID)
    .bind(&cust_exp)
    .bind(expired_at)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert verified case");

    let doc_passport_exp = Uuid::new_v4();
    let doc_nat_id_exp = Uuid::new_v4();
    let doc_other_exp = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
         checksum, mime, size_bytes, uploaded_at, uploaded_by, retention_until, created_at) \
         VALUES ($1, $4, $5, 'PASSPORT', 'ref-p-exp', '\\x1234', 'image/jpeg', 1024, now(), $6, NULL, now()), \
                ($2, $4, $5, 'NATIONAL_ID', 'ref-n-exp', '\\x5678', 'image/png', 2048, now(), $6, $7, now()), \
                ($3, $4, $5, 'OTHER', 'ref-o-exp', '\\x9abc', 'application/pdf', 512, now(), $6, $7, now())",
    )
    .bind(doc_passport_exp)
    .bind(doc_nat_id_exp)
    .bind(doc_other_exp)
    .bind(ORG_ID)
    .bind(case_exp)
    .bind(REVIEWER_USER_ID)
    .bind(known_other_retention)
    .execute(&db.pool)
    .await
    .expect("insert documents for expire case");

    let sweep_now = Utc::now();
    let expired_count = db
        .sweeper()
        .expire(sweep_now)
        .await
        .expect("expire verified case");
    assert_eq!(expired_count, 1);

    let p_ret_exp = db
        .retention_until(doc_passport_exp)
        .await
        .expect("PASSPORT retention_until must be set on expire");
    assert!(
        (p_ret_exp - (sweep_now + Duration::days(30)))
            .num_seconds()
            .abs()
            < 5,
        "PASSPORT retention_until must be ~30 days from expire, got {p_ret_exp}"
    );
    assert_eq!(
        db.retention_until(doc_nat_id_exp).await,
        None,
        "NATIONAL_ID retention_until must be set to NULL when its policy period is None"
    );
    assert_eq!(
        db.retention_until(doc_other_exp).await,
        Some(known_other_retention),
        "OTHER document_type retention_until must remain untouched on expire"
    );
}
