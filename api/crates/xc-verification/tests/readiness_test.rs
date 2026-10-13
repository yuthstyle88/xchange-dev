//! VF-6 (#1394) up-front tests: `Readiness` (verification readiness for booking and desk views).
//!
//! Locks design `docs/design/blueprint/verification.md` §9.1 VF-6, V12, and ticket #1394:
//! - `Readiness::of(customer)` answers `VERIFIED (until)` | `PENDING` | `NEEDS_UPDATE` | `EXPIRED` | `NONE`;
//! - customer with no case -> `NONE`;
//! - newest case is `DRAFT` -> `NONE` (lane controller ruling #1394);
//! - newest case is `SUBMITTED` or `UNDER_REVIEW` -> `PENDING`;
//! - newest case is `NEEDS_UPDATE` -> `NEEDS_UPDATE`;
//! - newest case is `EXPIRED` -> `EXPIRED`;
//! - newest case is `REJECTED` or `REVOKED` -> `NONE`;
//! - newest case is `VERIFIED` with future `expires_at` -> `VERIFIED { until }`;
//! - readiness reads from the newest case when multiple cases exist;
//! - `ReadinessService::of` matches `Readiness::of`.
//!
//! Written before the implementation by the test writer (gemini/studio-13, on the BE lane controller
//! claude/ad05bf8d's order); the implementer must not edit, delete or weaken them.

use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_verification::application::readiness::{Readiness, ReadinessService};

const MUST_FINISH: StdDuration = StdDuration::from_secs(30);
static STARTING: Semaphore = Semaphore::const_new(1);

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
        let schema = unique("vf6_rd");
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
"#,
                ))
                .execute(&pool)
                .await
                .expect("ensure verification tables");
            }
        }

        Self::seed(&pool).await;
        Self { pool, schema }
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

// ── 1. Customer with no case answers NONE ─────────────────────────────────────

#[tokio::test]
async fn customer_with_no_case_answers_none() {
    let db = Db::start().await;
    let cust_id = unique("cust_none");
    db.create_customer(&cust_id).await;

    // Customer with no case -> NONE (design §8, V12)
    let readiness = Readiness::of(&db.pool, &cust_id)
        .await
        .expect("readiness query");
    assert_eq!(readiness, Readiness::None);
}

// ── 2. Customer whose newest case is VERIFIED answers VERIFIED (until) ────────

#[tokio::test]
async fn customer_whose_newest_case_is_verified_answers_verified_until() {
    let db = Db::start().await;
    let cust_id = unique("cust_ver");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    let expires_at = Utc::now() + Duration::days(90);

    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'VERIFIED', 1, now(), now(), $4, $5, 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(expires_at)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert verified case");

    let readiness = Readiness::of(&db.pool, &cust_id)
        .await
        .expect("readiness query");

    match readiness {
        Readiness::Verified { until } => {
            let diff = (until - expires_at).num_seconds().abs();
            assert!(
                diff < 5,
                "until date matches stored expires_at: {diff}s difference"
            );
        }
        other => panic!("expected Readiness::Verified, got {other:?}"),
    }
}

// ── 3. Customer whose newest case is DRAFT answers NONE; SUBMITTED or UNDER_REVIEW answers PENDING ─

#[tokio::test]
async fn customer_whose_newest_case_is_draft_answers_none_and_submitted_or_under_review_answers_pending()
 {
    let db = Db::start().await;

    // DRAFT has sent nothing for review, so readiness is NONE (lane controller ruling #1394)
    let cust_draft = unique("cust_draft");
    db.create_customer(&cust_draft).await;
    let case_draft = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'DRAFT', 1, NULL, 1, now())",
    )
    .bind(case_draft)
    .bind(ORG_ID)
    .bind(&cust_draft)
    .execute(&db.pool)
    .await
    .expect("insert draft case");

    let readiness_draft = Readiness::of(&db.pool, &cust_draft)
        .await
        .expect("readiness query");
    assert_eq!(
        readiness_draft,
        Readiness::None,
        "status DRAFT must answer NONE"
    );

    // SUBMITTED and UNDER_REVIEW are pending desk action -> PENDING (V12)
    let pending_states = [("SUBMITTED", "cust_sub"), ("UNDER_REVIEW", "cust_ur")];

    for (status, prefix) in pending_states {
        let cust_id = unique(prefix);
        db.create_customer(&cust_id).await;
        let case_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
             submitted_at, version, created_at) \
             VALUES ($1, $2, $3, 'STAFF', $4, 1, now(), 1, now())",
        )
        .bind(case_id)
        .bind(ORG_ID)
        .bind(&cust_id)
        .bind(status)
        .execute(&db.pool)
        .await
        .expect("insert pending case");

        let readiness = Readiness::of(&db.pool, &cust_id)
            .await
            .expect("readiness query");
        assert_eq!(
            readiness,
            Readiness::Pending,
            "status {status} must answer PENDING"
        );
    }
}

// ── 4. Customer whose newest case is NEEDS_UPDATE answers NEEDS_UPDATE ─────────

#[tokio::test]
async fn customer_whose_newest_case_is_needs_update_answers_needs_update() {
    let db = Db::start().await;
    let cust_id = unique("cust_nu");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, decided_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'NEEDS_UPDATE', 1, now(), now(), $4, 'DETAILS_MISMATCH', 2, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert needs_update case");

    let readiness = Readiness::of(&db.pool, &cust_id)
        .await
        .expect("readiness query");
    assert_eq!(readiness, Readiness::NeedsUpdate);
}

// ── 5. Customer whose newest case is EXPIRED answers EXPIRED ──────────────────

#[tokio::test]
async fn customer_whose_newest_case_is_expired_answers_expired() {
    let db = Db::start().await;
    let cust_id = unique("cust_exp");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    let past = Utc::now() - Duration::days(5);

    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'EXPIRED', 1, now(), now(), $4, $5, 3, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(past)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert expired case");

    let readiness = Readiness::of(&db.pool, &cust_id)
        .await
        .expect("readiness query");
    assert_eq!(readiness, Readiness::Expired);
}

// ── 6. Customer whose newest case is REJECTED or REVOKED answers NONE ─────────

#[tokio::test]
async fn customer_whose_newest_case_is_rejected_or_revoked_answers_none() {
    let db = Db::start().await;
    let cust_rej = unique("cust_rej");
    let cust_rev = unique("cust_rev");
    db.create_customer(&cust_rej).await;
    db.create_customer(&cust_rev).await;

    let case_rej = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'REJECTED', 1, now(), $4, 'DOCUMENT_NOT_ACCEPTED', 2, now())",
    )
    .bind(case_rej)
    .bind(ORG_ID)
    .bind(&cust_rej)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert rejected case");

    let case_rev = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         decided_at, reviewed_by, decision_reason_code, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'REVOKED', 1, now(), $4, 'SUSPECTED_FRAUD', 2, now())",
    )
    .bind(case_rev)
    .bind(ORG_ID)
    .bind(&cust_rev)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert revoked case");

    // REJECTED and REVOKED end the case without an active verification -> NONE (V12)
    assert_eq!(
        Readiness::of(&db.pool, &cust_rej).await.expect("query"),
        Readiness::None,
        "rejected case -> NONE"
    );
    assert_eq!(
        Readiness::of(&db.pool, &cust_rev).await.expect("query"),
        Readiness::None,
        "revoked case -> NONE"
    );
}

// ── 7. Readiness reads from the newest case when multiple cases exist ─────────

#[tokio::test]
async fn readiness_reads_from_newest_case_when_multiple_cases_exist() {
    let db = Db::start().await;
    let cust_id = unique("cust_multi");
    db.create_customer(&cust_id).await;

    // Older case: EXPIRED 10 days ago
    let old_case = Uuid::new_v4();
    let old_created = Utc::now() - Duration::days(10);
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         submitted_at, decided_at, expires_at, reviewed_by, version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'EXPIRED', 1, $4, $4, $4, $5, 2, $4)",
    )
    .bind(old_case)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(old_created)
    .bind(REVIEWER_USER_ID)
    .execute(&db.pool)
    .await
    .expect("insert old expired case");

    // Before opening new case: customer readiness is EXPIRED
    let r_old = Readiness::of(&db.pool, &cust_id).await.expect("query");
    assert_eq!(r_old, Readiness::Expired, "old case determines readiness");

    // Newer case: Customer opens a renewal case (DRAFT, created 1 hour ago)
    let new_case = Uuid::new_v4();
    let new_created = Utc::now() - Duration::hours(1);
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'DRAFT', 1, 1, $4)",
    )
    .bind(new_case)
    .bind(ORG_ID)
    .bind(&cust_id)
    .bind(new_created)
    .execute(&db.pool)
    .await
    .expect("insert newer draft case");

    // V12 & lane controller ruling #1394: Readiness reads from the newest case -> DRAFT answers NONE
    let r_new = Readiness::of(&db.pool, &cust_id).await.expect("query");
    assert_eq!(
        r_new,
        Readiness::None,
        "readiness must reflect newest case (DRAFT -> NONE)"
    );
}

// ── 8. ReadinessService wrapper matches associated Readiness::of method ───────

#[tokio::test]
async fn readiness_service_wrapper_matches_associated_of_method() {
    let db = Db::start().await;
    let cust_id = unique("cust_svc");
    db.create_customer(&cust_id).await;

    let case_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
         version, created_at) \
         VALUES ($1, $2, $3, 'STAFF', 'SUBMITTED', 1, 1, now())",
    )
    .bind(case_id)
    .bind(ORG_ID)
    .bind(&cust_id)
    .execute(&db.pool)
    .await
    .expect("insert submitted case");

    let svc = ReadinessService::from_pool(db.pool.clone());
    let res_svc = svc.of(&cust_id).await.expect("svc query");
    let res_fn = Readiness::of(&db.pool, &cust_id).await.expect("fn query");

    assert_eq!(res_svc, res_fn);
    assert_eq!(res_svc, Readiness::Pending);
}
