//! Tests for 108-Plaza/xchange#1389:
//! A customer row with user_id NULL can have a verification case opened without error.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;
use xc_verification::application::case_service::CaseService;
use xc_verification::domain::case::case_actor::CaseActor;
use xc_verification::domain::case::case_status::CaseStatus;
use xc_verification::domain::case::customer_actor::CustomerActor;
use xc_verification::domain::case::provider::Provider;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const MUST_FINISH: Duration = Duration::from_secs(60);

static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

#[derive(Default)]
struct MemStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    puts: AtomicUsize,
}

impl DocumentStore for MemStore {
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, Result<String, DocumentStoreError>> {
        Box::pin(async move {
            self.puts.fetch_add(1, Ordering::Relaxed);
            self.objects
                .lock()
                .expect("not poisoned")
                .insert(doc_ref.to_owned(), bytes.to_vec());
            Ok("test-key-1".to_owned())
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

struct Db {
    pool: PgPool,
    schema: String,
    store: Arc<MemStore>,
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
        let schema = unique("vf3_null_user");
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
            .expect("drop the test schema");
        pool.close().await;
    }

    fn service(&self) -> CaseService {
        CaseService::new(
            self.pool.clone(),
            Arc::new(PgCaseRepository::new(self.pool.clone())),
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

    async fn branch(&self, org: &str) -> BranchId {
        let id = unique("B").replace('_', "-");
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{id}', '{id}', '{org}'::uuid)"
        ))
        .await;
        BranchId::new(id)
    }

    async fn branch_reviewer(&self, org: &str) -> ReviewerActor {
        let user_id = self.user(org).await;
        let branch_id = self.branch(org).await;
        ReviewerActor {
            user_id,
            scope: ReviewerScope::Branch {
                organization_id: Uuid::parse_str(org).expect("a uuid"),
                branch_id,
            },
        }
    }

    /// Create a customer with user_id NULL (e.g. walk-in desk customer with no app user).
    async fn customer_with_null_user(&self, org: &str) -> String {
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'Somchai', 'LIVE', now(), NULL)"
        ))
        .await;
        id
    }
}

#[tokio::test]
async fn customer_with_null_user_can_have_case_opened_by_branch_reviewer() {
    let db = Db::start().await;
    let customer_id = db.customer_with_null_user(ORG_1).await;
    let reviewer = db.branch_reviewer(ORG_1).await;
    let actor = CaseActor::Reviewer(reviewer.clone());
    let svc = db.service();

    let case = svc
        .open(&actor, &customer_id)
        .await
        .expect("open case for customer with user_id NULL must succeed");

    assert_eq!(case.status, CaseStatus::Draft);
    assert_eq!(case.policy_version, 1);
    assert_eq!(case.provider, Provider::Staff);
    assert_eq!(case.customer_id, customer_id);
    assert_eq!(case.organization_id.to_string(), ORG_1);
    match &reviewer.scope {
        ReviewerScope::Branch { branch_id, .. } => {
            assert_eq!(case.branch_id.as_ref(), Some(branch_id));
        }
        _ => panic!("expected branch scope"),
    }

    // Opening again returns the existing open case
    let second = svc
        .open(&actor, &customer_id)
        .await
        .expect("second open must return existing case");
    assert_eq!(second.id, case.id);

    db.finish().await;
}

#[tokio::test]
async fn customer_with_null_user_can_have_case_opened_by_customer_actor() {
    let db = Db::start().await;
    let customer_id = db.customer_with_null_user(ORG_1).await;
    let actor = CaseActor::Customer(CustomerActor {
        customer_id: customer_id.clone(),
    });
    let svc = db.service();

    let case = svc
        .open(&actor, &customer_id)
        .await
        .expect("open case by customer actor with user_id NULL must succeed");

    assert_eq!(case.status, CaseStatus::Draft);
    assert_eq!(case.policy_version, 1);
    assert_eq!(case.provider, Provider::Staff);
    assert_eq!(case.customer_id, customer_id);
    assert_eq!(case.organization_id.to_string(), ORG_1);
    assert_eq!(case.branch_id, None);

    db.finish().await;
}
