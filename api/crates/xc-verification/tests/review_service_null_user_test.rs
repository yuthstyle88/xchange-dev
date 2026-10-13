//! VF-4 #1391 rework:
//! Verify that when a customer has user_id NULL, an organisation reviewer
//! can successfully execute all five reviewer actions:
//! 1. start_review
//! 2. approve
//! 3. reject
//! 4. request_update
//! 5. revoke

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Days, NaiveDate};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_verification::application::review_service::ReviewService;
use xc_verification::domain::case::case_status::CaseStatus;
use xc_verification::domain::case::decision_reason::DecisionReason;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
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

struct Db {
    pool: PgPool,
    schema: String,
}

impl Db {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set");
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("vf4_null_user");
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
        }
        Self { pool, schema }
    }

    async fn finish(self) {
        let Db { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    fn service(&self) -> ReviewService {
        ReviewService::new(
            self.pool.clone(),
            Arc::new(PgCaseRepository::new(self.pool.clone())),
            Arc::new(PgAuditLog::new(self.pool.clone())),
        )
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn today_bangkok(&self) -> NaiveDate {
        let (d,): (NaiveDate,) = sqlx::query_as("SELECT (now() AT TIME ZONE 'Asia/Bangkok')::date")
            .fetch_one(&self.pool)
            .await
            .expect("today in bangkok");
        d
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

    async fn org_reviewer(&self, org: &str) -> ReviewerActor {
        let user_id = self.user(org).await;
        ReviewerActor {
            user_id,
            scope: ReviewerScope::Organization {
                organization_id: Uuid::from_str(org).expect("uuid"),
            },
        }
    }

    async fn customer_with_null_user(&self, org: &str) -> String {
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'Somchai', 'LIVE', now(), NULL)"
        ))
        .await;
        id
    }

    async fn insert_case(&self, org: &str, customer_id: &str, status: &str, version: i32) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, \
             policy_version, branch_id, submitted_at, decided_at, reviewed_by, decision_reason_code, \
             version, created_at) \
             VALUES ('{id}', '{org}'::uuid, '{customer_id}', 'STAFF', '{status}', 1, NULL, \
             now(), NULL, NULL, NULL, {version}, now())"
        ))
        .await;
        id
    }
}

#[tokio::test]
async fn customer_with_null_user_all_five_reviewer_actions_succeed_for_org_reviewer() {
    let db = Db::start().await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let customer_id = db.customer_with_null_user(ORG_1).await;
    let svc = db.service();

    // 1. start_review succeeds on SUBMITTED case
    let case_1 = db.insert_case(ORG_1, &customer_id, "SUBMITTED", 1).await;
    let started = svc
        .start_review(&reviewer, case_1, 1)
        .await
        .expect("start_review must succeed for customer with user_id NULL");
    assert_eq!(started.status, CaseStatus::UnderReview);
    assert_eq!(started.version, 2);

    // 2. approve succeeds on UNDER_REVIEW case
    let tomorrow = db.today_bangkok().await + Days::new(1);
    let approved = svc
        .approve(&reviewer, case_1, 2, tomorrow)
        .await
        .expect("approve must succeed for customer with user_id NULL");
    assert_eq!(approved.status, CaseStatus::Verified);
    assert_eq!(approved.version, 3);
    assert_eq!(approved.reviewed_by, Some(reviewer.user_id));

    // 3. revoke succeeds on VERIFIED case
    let revoked = svc
        .revoke(&reviewer, case_1, 3, DecisionReason::Other)
        .await
        .expect("revoke must succeed for customer with user_id NULL");
    assert_eq!(revoked.status, CaseStatus::Revoked);
    assert_eq!(revoked.version, 4);
    assert_eq!(revoked.reviewed_by, Some(reviewer.user_id));

    // 4. reject succeeds on UNDER_REVIEW case
    let case_2 = db.insert_case(ORG_1, &customer_id, "UNDER_REVIEW", 1).await;
    let rejected = svc
        .reject(&reviewer, case_2, 1, DecisionReason::SuspectedFraud)
        .await
        .expect("reject must succeed for customer with user_id NULL");
    assert_eq!(rejected.status, CaseStatus::Rejected);
    assert_eq!(rejected.version, 2);
    assert_eq!(rejected.reviewed_by, Some(reviewer.user_id));

    // 5. request_update succeeds on UNDER_REVIEW case
    let case_3 = db.insert_case(ORG_1, &customer_id, "UNDER_REVIEW", 1).await;
    let requested = svc
        .request_update(&reviewer, case_3, 1, DecisionReason::DocumentUnreadable)
        .await
        .expect("request_update must succeed for customer with user_id NULL");
    assert_eq!(requested.status, CaseStatus::NeedsUpdate);
    assert_eq!(requested.version, 2);
    assert_eq!(requested.reviewed_by, Some(reviewer.user_id));

    db.finish().await;
}
