//! VF-4 #1391 — the reviewer side of a verification case: `ReviewService` (start review, decide),
//! `ReviewError`, and reviewer-side transitions (design `docs/design/blueprint/verification.md`
//! V4, V5, V6, V14, V17, §4.2, §4.3, §4.5 F02, §5, §10, §12 S-5; §9.1 row VF-4; ticket #1391 §4, §8, §9).
//! Written before the implementation by the test writer (gemini/studio-13, on the BE lane controller
//! claude/ad05bf8d's order, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The database comes from every migration in `api/migrations/` (VF-1's M1 `<version>_verification.sql` included,
//! which seeds policy version 1 for every organisation existing at M1). Each test works in a schema of its own.
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Days, NaiveDate, Utc};
use futures::future::BoxFuture;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;
use xc_verification::application::case_service::CaseService;
use xc_verification::application::review_service::{ReviewError, ReviewService};
use xc_verification::domain::case::case_actor::CaseActor;
use xc_verification::domain::case::case_status::CaseStatus;
use xc_verification::domain::case::customer_actor::CustomerActor;
use xc_verification::domain::case::decision_reason::DecisionReason;
use xc_verification::domain::case::provider::Provider;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

/// The organisation OR-1's migration creates; M1 seeds its policy version 1.
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

// ── In-memory DocumentStore ──────────────────────────────────────────────────

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
            Ok("test-key".to_owned())
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

// ── test database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    schema: String,
}

struct Customer {
    id: String,
    user: Uuid,
}

impl Customer {
    fn actor(&self) -> CaseActor {
        CaseActor::Customer(CustomerActor {
            customer_id: self.id.clone(),
        })
    }
}

impl Db {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The verification tests need a real PostgreSQL and do not skip; \
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
        let schema = unique("vf4");
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

    fn case_service(&self) -> CaseService {
        CaseService::new(
            self.pool.clone(),
            Arc::new(PgCaseRepository::new(self.pool.clone())),
            Arc::new(MemStore::default()),
            Arc::new(PgAuditLog::new(self.pool.clone())),
        )
    }

    async fn today_bangkok(&self) -> NaiveDate {
        let (d,): (NaiveDate,) = sqlx::query_as("SELECT (now() AT TIME ZONE 'Asia/Bangkok')::date")
            .fetch_one(&self.pool)
            .await
            .expect("today in bangkok");
        d
    }

    async fn insert_document(&self, org: &str, case_id: Uuid, user_id: Uuid) -> Uuid {
        let doc_id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO verification_documents (id, organization_id, case_id, document_type, \
             encrypted_object_ref, key_id, checksum, mime, size_bytes, uploaded_at, uploaded_by, \
             access_class, created_at) \
             VALUES ('{doc_id}', '{org}'::uuid, '{case_id}', 'PASSPORT', 'ref-{doc_id}', 'k1', \
             '\\x01020304', 'image/png', 100, now(), '{user_id}', 'IDENTITY_DOCUMENT', now())"
        ))
        .await;
        doc_id
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

    async fn text(&self, sql: &str) -> Option<String> {
        let row: Option<(Option<String>,)> = sqlx::query_as(AssertSqlSafe(sql.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        row.and_then(|(t,)| t)
    }

    async fn organisation(&self) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('{id}', '{}', 'Asia/Bangkok', now())",
            unique("org")
        ))
        .await;
        // Seed policy v1 for the new organisation
        self.exec(&format!(
            "INSERT INTO verification_policies (organization_id, version, accepted_document_types, \
             documents_required, branch_review_allowed, published_by, published_at, created_at) \
             VALUES ('{id}', 1, ARRAY['PASSPORT','NATIONAL_ID'], 1, true, NULL, now(), now())"
        ))
        .await;
        id
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
        Customer { id, user }
    }

    async fn customer_with_user(&self, org: &str, user: Uuid) -> Customer {
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'Somchai', 'LIVE', now(), '{user}')"
        ))
        .await;
        Customer { id, user }
    }

    async fn branch(&self, org: &str) -> BranchId {
        let id = unique("B").replace('_', "-");
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{id}', '{id}', '{org}'::uuid)"
        ))
        .await;
        BranchId::new(id)
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

    async fn branch_reviewer(&self, org: &str, branch_id: BranchId) -> ReviewerActor {
        let user_id = self.user(org).await;
        ReviewerActor {
            user_id,
            scope: ReviewerScope::Branch {
                organization_id: Uuid::from_str(org).expect("uuid"),
                branch_id,
            },
        }
    }

    async fn insert_case(
        &self,
        org: &str,
        customer_id: &str,
        status: &str,
        branch_id: Option<&BranchId>,
        version: i32,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let branch_val = match branch_id {
            Some(b) => format!("'{}'", b.as_str()),
            None => "NULL".to_string(),
        };
        let (reviewed_by, decided_at, reason_code) = match status {
            "VERIFIED" => {
                let u = self.user(org).await;
                (format!("'{u}'::uuid"), "now()", "NULL")
            }
            "REJECTED" | "NEEDS_UPDATE" | "REVOKED" => {
                let u = self.user(org).await;
                (format!("'{u}'::uuid"), "now()", "'OTHER'")
            }
            _ => ("NULL".to_string(), "NULL", "NULL"),
        };
        self.exec(&format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, \
             policy_version, branch_id, submitted_at, decided_at, reviewed_by, decision_reason_code, \
             version, created_at) \
             VALUES ('{id}', '{org}'::uuid, '{customer_id}', 'STAFF', '{status}', 1, {branch_val}, \
             now(), {decided_at}, {reviewed_by}, {reason_code}, {version}, now())"
        ))
        .await;
        id
    }

    #[allow(dead_code)]
    async fn audit_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM audit_events").await
    }

    async fn audit_events_for_case(
        &self,
        case_id: Uuid,
    ) -> Vec<(String, String, String, String, Option<Value>, Option<Value>)> {
        sqlx::query_as::<_, (String, String, String, String, Option<Value>, Option<Value>)>(
            "SELECT actor_id, actor_role, action, object_ref, before_redacted, after_redacted \
             FROM audit_events WHERE object_ref LIKE $1 ORDER BY id ASC",
        )
        .bind(format!("%{case_id}%"))
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
    }
}

fn assert_audit_event(
    event: &(String, String, String, String, Option<Value>, Option<Value>),
    expected_actor_id: Uuid,
    expected_case_id: Uuid,
    expected_action_keyword: &str,
    expected_before_status: &str,
    expected_after_status: &str,
) {
    let (actor_id, actor_role, action, object_ref, before, after) = event;
    assert_eq!(
        actor_id,
        &expected_actor_id.to_string(),
        "actor_id must match reviewer"
    );
    assert_eq!(
        actor_role, "VERIFICATION_REVIEWER",
        "actor_role must be VERIFICATION_REVIEWER"
    );
    assert!(
        action.to_lowercase().contains(expected_action_keyword),
        "action '{action}' must contain '{expected_action_keyword}'"
    );
    assert!(
        object_ref.contains(&expected_case_id.to_string()),
        "object_ref '{object_ref}' must reference case {expected_case_id}"
    );
    assert_eq!(
        before
            .as_ref()
            .and_then(|v| v.get("status"))
            .and_then(|s| s.as_str()),
        Some(expected_before_status),
        "before status must be {expected_before_status}"
    );
    assert_eq!(
        after
            .as_ref()
            .and_then(|v| v.get("status"))
            .and_then(|s| s.as_str()),
        Some(expected_after_status),
        "after status must be {expected_after_status}"
    );
}

// ── 1. Concurrency: Two reviewers act on one case with same expectedVersion ──

#[tokio::test]
async fn two_reviewers_act_with_same_expected_version_one_wins_other_gets_stale_version() {
    let db = Db::start().await;
    let customer = db.customer(ORG_1).await;
    let case_id = db
        .insert_case(ORG_1, &customer.id, "SUBMITTED", None, 1)
        .await;

    let rev1 = db.org_reviewer(ORG_1).await;
    let rev2 = db.org_reviewer(ORG_1).await;

    let svc1 = db.service();
    let svc2 = db.service();

    let (r1, r2) = tokio::join!(
        svc1.start_review(&rev1, case_id, 1),
        svc2.start_review(&rev2, case_id, 1),
    );

    let (ok, err) = match (r1, r2) {
        (Ok(c), Err(e)) => (c, e),
        (Err(e), Ok(c)) => (c, e),
        (Ok(_), Ok(_)) => {
            panic!("both reviewers succeeded; expectedVersion CAS must let exactly one win")
        }
        (Err(e1), Err(e2)) => panic!("both reviewers failed: {e1:?}, {e2:?}"),
    };

    assert_eq!(ok.status, CaseStatus::UnderReview);
    assert_eq!(ok.version, 2);
    assert_eq!(err.code(), "staleVersion");
    assert_eq!(err, ReviewError::StaleVersion);

    // Verify DB reflects exactly one review transition
    let status = db
        .text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await
        .expect("status");
    assert_eq!(status, "UNDER_REVIEW");

    let version = db
        .count(&format!(
            "SELECT version::int8 FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await;
    assert_eq!(version, 2);

    db.finish().await;
}

// ── 2. Self-review: Reviewer whose user is the case's customer is outOfScope ─

#[tokio::test]
async fn self_review_is_strictly_prohibited_on_all_reviewer_actions() {
    let db = Db::start().await;
    let svc = db.service();

    // 1) start_review: self-review refused
    let u1 = db.user(ORG_1).await;
    let c1 = db.customer_with_user(ORG_1, u1).await;
    let r1 = ReviewerActor {
        user_id: u1,
        scope: ReviewerScope::Organization {
            organization_id: Uuid::from_str(ORG_1).expect("uuid"),
        },
    };
    let case_sub = db.insert_case(ORG_1, &c1.id, "SUBMITTED", None, 1).await;
    let err = svc.start_review(&r1, case_sub, 1).await.unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "self-review on start_review is outOfScope"
    );
    assert_eq!(err, ReviewError::OutOfScope);

    // 2) approve: self-review refused
    let u2 = db.user(ORG_1).await;
    let c2 = db.customer_with_user(ORG_1, u2).await;
    let r2 = ReviewerActor {
        user_id: u2,
        scope: ReviewerScope::Organization {
            organization_id: Uuid::from_str(ORG_1).expect("uuid"),
        },
    };
    let case_ur = db.insert_case(ORG_1, &c2.id, "UNDER_REVIEW", None, 2).await;
    let today = db.today_bangkok().await;
    let tomorrow = today + Days::new(1);
    let err = svc.approve(&r2, case_ur, 2, tomorrow).await.unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "self-review on approve is outOfScope"
    );
    assert_eq!(err, ReviewError::OutOfScope);

    // 3) reject: self-review refused
    let u3 = db.user(ORG_1).await;
    let c3 = db.customer_with_user(ORG_1, u3).await;
    let r3 = ReviewerActor {
        user_id: u3,
        scope: ReviewerScope::Organization {
            organization_id: Uuid::from_str(ORG_1).expect("uuid"),
        },
    };
    let case_rej = db.insert_case(ORG_1, &c3.id, "UNDER_REVIEW", None, 2).await;
    let err = svc
        .reject(&r3, case_rej, 2, DecisionReason::Other)
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "self-review on reject is outOfScope"
    );
    assert_eq!(err, ReviewError::OutOfScope);

    // 4) request_update: self-review refused
    let u4 = db.user(ORG_1).await;
    let c4 = db.customer_with_user(ORG_1, u4).await;
    let r4 = ReviewerActor {
        user_id: u4,
        scope: ReviewerScope::Organization {
            organization_id: Uuid::from_str(ORG_1).expect("uuid"),
        },
    };
    let case_nu = db.insert_case(ORG_1, &c4.id, "UNDER_REVIEW", None, 2).await;
    let err = svc
        .request_update(&r4, case_nu, 2, DecisionReason::Other)
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "self-review on request_update is outOfScope"
    );
    assert_eq!(err, ReviewError::OutOfScope);

    // 5) revoke: self-review refused
    let u5 = db.user(ORG_1).await;
    let c5 = db.customer_with_user(ORG_1, u5).await;
    let r5 = ReviewerActor {
        user_id: u5,
        scope: ReviewerScope::Organization {
            organization_id: Uuid::from_str(ORG_1).expect("uuid"),
        },
    };
    let case_ver = db.insert_case(ORG_1, &c5.id, "VERIFIED", None, 3).await;
    let err = svc
        .revoke(&r5, case_ver, 3, DecisionReason::SuspectedFraud)
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "self-review on revoke is outOfScope"
    );
    assert_eq!(err, ReviewError::OutOfScope);

    db.finish().await;
}

// ── 3. Decision reasons: All six S-5 codes accepted and read back from DB ─────

#[tokio::test]
async fn reject_request_update_and_revoke_succeed_with_all_six_decided_reasons() {
    let db = Db::start().await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    // S-5 owner decision: DOCUMENT_UNREADABLE, DOCUMENT_EXPIRED, DETAILS_MISMATCH,
    // DOCUMENT_NOT_ACCEPTED, SUSPECTED_FRAUD, OTHER.
    for reason in DecisionReason::ALL {
        // 1) reject accepts reason and stores it in DB
        let c = db.customer(ORG_1).await;
        let case_id = db.insert_case(ORG_1, &c.id, "UNDER_REVIEW", None, 1).await;
        let res = svc
            .reject(&reviewer, case_id, 1, reason)
            .await
            .expect("reject with valid reason");
        assert_eq!(res.status, CaseStatus::Rejected);
        assert_eq!(res.decision_reason, Some(reason));
        assert_eq!(res.version, 2);

        let (status, reason_code, reviewed_by, decided_at, version): (
            String,
            Option<String>,
            Option<Uuid>,
            Option<DateTime<Utc>>,
            i32,
        ) = sqlx::query_as(
            "SELECT status, decision_reason_code, reviewed_by, decided_at, version \
             FROM verification_cases WHERE id = $1",
        )
        .bind(case_id)
        .fetch_one(&db.pool)
        .await
        .expect("read back rejected case from DB");
        assert_eq!(status, "REJECTED");
        assert_eq!(reason_code.as_deref(), Some(reason.as_str()));
        assert_eq!(reviewed_by, Some(reviewer.user_id));
        assert!(decided_at.is_some());
        assert_eq!(version, 2);

        // 2) request_update accepts reason and stores it in DB
        let c = db.customer(ORG_1).await;
        let case_id = db.insert_case(ORG_1, &c.id, "UNDER_REVIEW", None, 1).await;
        let res = svc
            .request_update(&reviewer, case_id, 1, reason)
            .await
            .expect("request_update with valid reason");
        assert_eq!(res.status, CaseStatus::NeedsUpdate);
        assert_eq!(res.decision_reason, Some(reason));
        assert_eq!(res.version, 2);

        let (status, reason_code, reviewed_by, decided_at, version): (
            String,
            Option<String>,
            Option<Uuid>,
            Option<DateTime<Utc>>,
            i32,
        ) = sqlx::query_as(
            "SELECT status, decision_reason_code, reviewed_by, decided_at, version \
             FROM verification_cases WHERE id = $1",
        )
        .bind(case_id)
        .fetch_one(&db.pool)
        .await
        .expect("read back needs_update case from DB");
        assert_eq!(status, "NEEDS_UPDATE");
        assert_eq!(reason_code.as_deref(), Some(reason.as_str()));
        assert_eq!(reviewed_by, Some(reviewer.user_id));
        assert!(decided_at.is_some());
        assert_eq!(version, 2);

        // 3) revoke accepts reason and stores it in DB
        let c = db.customer(ORG_1).await;
        let case_id = db.insert_case(ORG_1, &c.id, "VERIFIED", None, 1).await;
        let res = svc
            .revoke(&reviewer, case_id, 1, reason)
            .await
            .expect("revoke with valid reason");
        assert_eq!(res.status, CaseStatus::Revoked);
        assert_eq!(res.decision_reason, Some(reason));
        assert_eq!(res.version, 2);

        let (status, reason_code, reviewed_by, decided_at, version): (
            String,
            Option<String>,
            Option<Uuid>,
            Option<DateTime<Utc>>,
            i32,
        ) = sqlx::query_as(
            "SELECT status, decision_reason_code, reviewed_by, decided_at, version \
             FROM verification_cases WHERE id = $1",
        )
        .bind(case_id)
        .fetch_one(&db.pool)
        .await
        .expect("read back revoked case from DB");
        assert_eq!(status, "REVOKED");
        assert_eq!(reason_code.as_deref(), Some(reason.as_str()));
        assert_eq!(reviewed_by, Some(reviewer.user_id));
        assert!(decided_at.is_some());
        assert_eq!(version, 2);
    }

    db.finish().await;
}

// ── 4. Lifecycle: request_update -> NEEDS_UPDATE, resubmit -> SUBMITTED ─────

#[tokio::test]
async fn request_update_moves_to_needs_update_and_resubmit_moves_to_submitted() {
    let db = Db::start().await;
    let customer = db.customer(ORG_1).await;
    let case_id = db
        .insert_case(ORG_1, &customer.id, "UNDER_REVIEW", None, 2)
        .await;
    // Attach document so customer resubmit satisfies documents policy
    db.insert_document(ORG_1, case_id, customer.user).await;

    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    // 1) Reviewer requests update
    let updated = svc
        .request_update(&reviewer, case_id, 2, DecisionReason::DocumentUnreadable)
        .await
        .expect("request_update");
    assert_eq!(updated.status, CaseStatus::NeedsUpdate);
    assert_eq!(updated.version, 3);
    assert_eq!(
        updated.decision_reason,
        Some(DecisionReason::DocumentUnreadable)
    );
    assert_eq!(updated.reviewed_by, Some(reviewer.user_id));

    // Verify DB reflects NEEDS_UPDATE
    let status_in_db = db
        .text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await
        .expect("status");
    assert_eq!(status_in_db, "NEEDS_UPDATE");

    // 2) Customer resubmits through VF-3's CaseService (not raw SQL UPDATE)
    let case_svc = db.case_service();
    let resubmitted = case_svc
        .submit(&customer.actor(), case_id)
        .await
        .expect("customer resubmit via CaseService");
    assert_eq!(resubmitted.status, CaseStatus::Submitted);

    let resubmitted_status = db
        .text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await
        .expect("status");
    assert_eq!(resubmitted_status, "SUBMITTED");

    // 3) Reviewer can start review again on the resubmitted case using resubmitted.version
    let review_again = svc
        .start_review(&reviewer, case_id, resubmitted.version)
        .await
        .expect("start_review after resubmit");
    assert_eq!(review_again.status, CaseStatus::UnderReview);
    assert_eq!(review_again.version, resubmitted.version + 1);

    db.finish().await;
}

// ── 5. Audit: Every action writes one row, rolled-back writes none ───────────

#[tokio::test]
async fn every_action_writes_one_audit_row_and_rolled_back_action_writes_none() {
    let db = Db::start().await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();
    let today = db.today_bangkok().await;
    let tomorrow = today + Days::new(1);

    // ─────────────────────────────────────────────────────────────────────────
    // 1) start_review, approve, revoke on case 1
    // ─────────────────────────────────────────────────────────────────────────
    let customer1 = db.customer(ORG_1).await;
    let case1 = db
        .insert_case(ORG_1, &customer1.id, "SUBMITTED", None, 1)
        .await;

    // 1.1) start_review writes one row
    svc.start_review(&reviewer, case1, 1)
        .await
        .expect("start_review");
    let events = db.audit_events_for_case(case1).await;
    assert_eq!(
        events.len(),
        1,
        "start_review must write exactly 1 audit row"
    );
    assert_audit_event(
        &events[0],
        reviewer.user_id,
        case1,
        "review",
        "SUBMITTED",
        "UNDER_REVIEW",
    );

    // 1.2) Rolled back action (staleVersion) writes NO audit row
    let err = svc.start_review(&reviewer, case1, 1).await.unwrap_err();
    assert_eq!(err, ReviewError::StaleVersion);
    let events = db.audit_events_for_case(case1).await;
    assert_eq!(
        events.len(),
        1,
        "rolled back action (staleVersion) writes no row"
    );

    // 1.3) Rolled back action (validationFailed with today date) writes NO audit row
    let err = svc.approve(&reviewer, case1, 2, today).await.unwrap_err();
    assert_eq!(err, ReviewError::ValidationFailed);
    let events = db.audit_events_for_case(case1).await;
    assert_eq!(events.len(), 1, "failed approval writes no audit row");

    // 1.4) approve writes one row (UNDER_REVIEW -> VERIFIED)
    svc.approve(&reviewer, case1, 2, tomorrow)
        .await
        .expect("approve");
    let events = db.audit_events_for_case(case1).await;
    assert_eq!(events.len(), 2, "approve must write exactly 1 audit row");
    assert_audit_event(
        &events[1],
        reviewer.user_id,
        case1,
        "approve",
        "UNDER_REVIEW",
        "VERIFIED",
    );

    // 1.5) revoke writes one row (VERIFIED -> REVOKED)
    svc.revoke(&reviewer, case1, 3, DecisionReason::Other)
        .await
        .expect("revoke");
    let events = db.audit_events_for_case(case1).await;
    assert_eq!(events.len(), 3, "revoke must write exactly 1 audit row");
    assert_audit_event(
        &events[2],
        reviewer.user_id,
        case1,
        "revoke",
        "VERIFIED",
        "REVOKED",
    );

    // ─────────────────────────────────────────────────────────────────────────
    // 2) reject on case 2
    // ─────────────────────────────────────────────────────────────────────────
    let customer2 = db.customer(ORG_1).await;
    let case2 = db
        .insert_case(ORG_1, &customer2.id, "SUBMITTED", None, 1)
        .await;

    svc.start_review(&reviewer, case2, 1)
        .await
        .expect("start_review case 2");
    svc.reject(&reviewer, case2, 2, DecisionReason::SuspectedFraud)
        .await
        .expect("reject");
    let events2 = db.audit_events_for_case(case2).await;
    assert_eq!(events2.len(), 2, "start_review + reject write 2 rows");
    assert_audit_event(
        &events2[1],
        reviewer.user_id,
        case2,
        "reject",
        "UNDER_REVIEW",
        "REJECTED",
    );

    // ─────────────────────────────────────────────────────────────────────────
    // 3) request_update on case 3
    // ─────────────────────────────────────────────────────────────────────────
    let customer3 = db.customer(ORG_1).await;
    let case3 = db
        .insert_case(ORG_1, &customer3.id, "SUBMITTED", None, 1)
        .await;

    svc.start_review(&reviewer, case3, 1)
        .await
        .expect("start_review case 3");
    svc.request_update(&reviewer, case3, 2, DecisionReason::DocumentUnreadable)
        .await
        .expect("request_update");
    let events3 = db.audit_events_for_case(case3).await;
    assert_eq!(
        events3.len(),
        2,
        "start_review + request_update write 2 rows"
    );
    assert_audit_event(
        &events3[1],
        reviewer.user_id,
        case3,
        "update",
        "UNDER_REVIEW",
        "NEEDS_UPDATE",
    );

    db.finish().await;
}

// ── 6. Expiry date: Today or past date refused as validationFailed ────────────

#[tokio::test]
async fn approve_with_today_or_past_date_is_refused_as_validation_failed() {
    let db = Db::start().await;
    let customer = db.customer(ORG_1).await;
    let case_id = db
        .insert_case(ORG_1, &customer.id, "UNDER_REVIEW", None, 1)
        .await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    let today = db.today_bangkok().await;
    let yesterday = today - Days::new(1);
    let past = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();

    for bad_date in [today, yesterday, past] {
        let err = svc
            .approve(&reviewer, case_id, 1, bad_date)
            .await
            .unwrap_err();
        assert_eq!(
            err.code(),
            "validationFailed",
            "date {bad_date} must be validationFailed"
        );
        assert_eq!(err, ReviewError::ValidationFailed);
    }

    // Case is still UNDER_REVIEW and version is still 1
    let status = db
        .text(&format!(
            "SELECT status FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await
        .expect("status");
    assert_eq!(status, "UNDER_REVIEW");

    let version = db
        .count(&format!(
            "SELECT version::int8 FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await;
    assert_eq!(version, 1);

    db.finish().await;
}

// ── 7. Expiry date: Future date stores expires_at ─────────────────────────────

#[tokio::test]
async fn approve_with_future_date_stores_expires_at() {
    let db = Db::start().await;
    let customer = db.customer(ORG_1).await;
    let case_id = db
        .insert_case(ORG_1, &customer.id, "UNDER_REVIEW", None, 1)
        .await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    let future = db.today_bangkok().await + Days::new(365);
    let approved = svc
        .approve(&reviewer, case_id, 1, future)
        .await
        .expect("approve");

    assert_eq!(approved.status, CaseStatus::Verified);
    assert_eq!(approved.version, 2);
    assert_eq!(approved.reviewed_by, Some(reviewer.user_id));
    assert!(approved.decided_at.is_some());
    assert!(approved.expires_at.is_some());

    // Check stored expires_at in DB
    let expires_at_db = db
        .text(&format!("SELECT to_char(expires_at, 'YYYY-MM-DD') FROM verification_cases WHERE id = '{case_id}'"))
        .await
        .expect("expires_at");
    assert_eq!(
        expires_at_db,
        future.to_string(),
        "stores the entered date as expires_at"
    );

    // Read back full record to verify stored fields
    let (db_status, db_reviewed_by, db_decided_at, db_version): (
        String,
        Option<Uuid>,
        Option<DateTime<Utc>>,
        i32,
    ) = sqlx::query_as(
        "SELECT status, reviewed_by, decided_at, version \
         FROM verification_cases WHERE id = $1",
    )
    .bind(case_id)
    .fetch_one(&db.pool)
    .await
    .expect("read back approved case");
    assert_eq!(db_status, "VERIFIED");
    assert_eq!(db_reviewed_by, Some(reviewer.user_id));
    assert!(db_decided_at.is_some());
    assert_eq!(db_version, 2);

    db.finish().await;
}

// ── 8. Provider: provider stays STAFF across all actions ─────────────────────

#[tokio::test]
async fn provider_stays_staff_across_all_reviewer_actions() {
    let db = Db::start().await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    // 1) start_review
    let c1 = db.customer(ORG_1).await;
    let case_id = db.insert_case(ORG_1, &c1.id, "SUBMITTED", None, 1).await;
    let c = svc
        .start_review(&reviewer, case_id, 1)
        .await
        .expect("start_review");
    assert_eq!(c.provider, Provider::Staff);

    // 2) request_update
    let c = svc
        .request_update(&reviewer, case_id, 2, DecisionReason::Other)
        .await
        .expect("request_update");
    assert_eq!(c.provider, Provider::Staff);

    // 3) reject
    let c2 = db.customer(ORG_1).await;
    let case_id2 = db.insert_case(ORG_1, &c2.id, "UNDER_REVIEW", None, 1).await;
    let c = svc
        .reject(&reviewer, case_id2, 1, DecisionReason::DocumentExpired)
        .await
        .expect("reject");
    assert_eq!(c.provider, Provider::Staff);

    // 4) approve
    let c3 = db.customer(ORG_1).await;
    let case_id3 = db.insert_case(ORG_1, &c3.id, "UNDER_REVIEW", None, 1).await;
    let tomorrow = db.today_bangkok().await + Days::new(1);
    let c = svc
        .approve(&reviewer, case_id3, 1, tomorrow)
        .await
        .expect("approve");
    assert_eq!(c.provider, Provider::Staff);

    // 5) revoke
    let c = svc
        .revoke(&reviewer, case_id3, 2, DecisionReason::SuspectedFraud)
        .await
        .expect("revoke");
    assert_eq!(c.provider, Provider::Staff);

    // Verify all cases in DB have provider = 'STAFF'
    let non_staff = db
        .count("SELECT count(*) FROM verification_cases WHERE provider != 'STAFF'")
        .await;
    assert_eq!(non_staff, 0, "provider column never deviates from STAFF");

    db.finish().await;
}

// ── 9. Edge case: Revoke on non-VERIFIED case refused ────────────────────────

#[tokio::test]
async fn revoke_on_non_verified_case_is_refused() {
    let db = Db::start().await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    let non_verified_statuses = [
        "DRAFT",
        "SUBMITTED",
        "UNDER_REVIEW",
        "REJECTED",
        "NEEDS_UPDATE",
        "EXPIRED",
        "REVOKED",
    ];

    for st in non_verified_statuses {
        let customer = db.customer(ORG_1).await;
        let case_id = db.insert_case(ORG_1, &customer.id, st, None, 1).await;
        let err = svc
            .revoke(&reviewer, case_id, 1, DecisionReason::Other)
            .await
            .unwrap_err();
        assert_eq!(
            err.code(),
            "outOfScope",
            "revoke on {st} must be refused as outOfScope"
        );
        assert_eq!(err, ReviewError::OutOfScope);
    }

    // Revoke on VERIFIED case succeeds
    let customer = db.customer(ORG_1).await;
    let verified_id = db
        .insert_case(ORG_1, &customer.id, "VERIFIED", None, 1)
        .await;
    let revoked = svc
        .revoke(&reviewer, verified_id, 1, DecisionReason::Other)
        .await
        .expect("revoke on VERIFIED");
    assert_eq!(revoked.status, CaseStatus::Revoked);

    db.finish().await;
}

// ── 10. Edge case: Reviewer from another organisation is outOfScope ──────────

#[tokio::test]
async fn reviewer_from_another_organisation_is_out_of_scope() {
    let db = Db::start().await;
    let org_b = db.organisation().await.to_string();

    let c1 = db.customer(ORG_1).await;
    let reviewer_b = db.org_reviewer(&org_b).await;
    let svc = db.service();

    let case_sub = db.insert_case(ORG_1, &c1.id, "SUBMITTED", None, 1).await;
    let err = svc
        .start_review(&reviewer_b, case_sub, 1)
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "reviewer from another org gets outOfScope"
    );
    assert_eq!(err, ReviewError::OutOfScope);

    let c2 = db.customer(ORG_1).await;
    let case_ur = db.insert_case(ORG_1, &c2.id, "UNDER_REVIEW", None, 1).await;
    let tomorrow = db.today_bangkok().await + Days::new(1);
    assert_eq!(
        svc.approve(&reviewer_b, case_ur, 1, tomorrow)
            .await
            .unwrap_err()
            .code(),
        "outOfScope"
    );
    assert_eq!(
        svc.reject(&reviewer_b, case_ur, 1, DecisionReason::Other)
            .await
            .unwrap_err()
            .code(),
        "outOfScope"
    );
    assert_eq!(
        svc.request_update(&reviewer_b, case_ur, 1, DecisionReason::Other)
            .await
            .unwrap_err()
            .code(),
        "outOfScope"
    );

    let c3 = db.customer(ORG_1).await;
    let case_ver = db.insert_case(ORG_1, &c3.id, "VERIFIED", None, 1).await;
    assert_eq!(
        svc.revoke(&reviewer_b, case_ver, 1, DecisionReason::Other)
            .await
            .unwrap_err()
            .code(),
        "outOfScope"
    );

    db.finish().await;
}

// ── 11. V17: Branch-scoped reviewer only acts on their branch ────────────────

#[tokio::test]
async fn branch_scoped_reviewer_only_acts_on_their_branch() {
    let db = Db::start().await;
    let branch_1 = db.branch(ORG_1).await;
    let branch_2 = db.branch(ORG_1).await;

    let branch_reviewer = db.branch_reviewer(ORG_1, branch_1.clone()).await;
    let org_reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    // 1) Case for Branch 2: Branch 1 reviewer gets outOfScope
    let customer_b2 = db.customer(ORG_1).await;
    let case_b2 = db
        .insert_case(ORG_1, &customer_b2.id, "SUBMITTED", Some(&branch_2), 1)
        .await;
    let err = svc
        .start_review(&branch_reviewer, case_b2, 1)
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "branch 1 reviewer cannot touch branch 2 case"
    );

    // 2) App case (branch_id = NULL): Branch reviewer cannot act on org queue
    let customer_app = db.customer(ORG_1).await;
    let case_app = db
        .insert_case(ORG_1, &customer_app.id, "SUBMITTED", None, 1)
        .await;
    let err = svc
        .start_review(&branch_reviewer, case_app, 1)
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        "outOfScope",
        "branch reviewer cannot touch app case"
    );

    // Org reviewer CAN act on app case
    let started_app = svc
        .start_review(&org_reviewer, case_app, 1)
        .await
        .expect("org reviewer on app case");
    assert_eq!(started_app.status, CaseStatus::UnderReview);

    // 3) Case for Branch 1: Branch 1 reviewer CAN act
    let customer_b1 = db.customer(ORG_1).await;
    let case_b1 = db
        .insert_case(ORG_1, &customer_b1.id, "SUBMITTED", Some(&branch_1), 1)
        .await;
    let started_b1 = svc
        .start_review(&branch_reviewer, case_b1, 1)
        .await
        .expect("branch reviewer on branch 1 case");
    assert_eq!(started_b1.status, CaseStatus::UnderReview);

    db.finish().await;
}

// ── 12. CaseMachine: Wrong state for reviewer action is refused ──────────────

#[tokio::test]
async fn reviewer_actions_on_invalid_states_are_refused() {
    let db = Db::start().await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    // start_review only from SUBMITTED; DRAFT, VERIFIED, REJECTED refused
    for st in ["DRAFT", "VERIFIED", "REJECTED"] {
        let c = db.customer(ORG_1).await;
        let case_id = db.insert_case(ORG_1, &c.id, st, None, 1).await;
        let err = svc.start_review(&reviewer, case_id, 1).await.unwrap_err();
        assert_eq!(
            err.code(),
            "outOfScope",
            "start_review on {st} must be outOfScope"
        );
    }

    // approve, reject, request_update only from UNDER_REVIEW
    let tomorrow = db.today_bangkok().await + Days::new(1);
    for st in ["SUBMITTED", "DRAFT", "REJECTED"] {
        let c = db.customer(ORG_1).await;
        let case_id = db.insert_case(ORG_1, &c.id, st, None, 1).await;
        assert_eq!(
            svc.approve(&reviewer, case_id, 1, tomorrow)
                .await
                .unwrap_err()
                .code(),
            "outOfScope"
        );
        assert_eq!(
            svc.reject(&reviewer, case_id, 1, DecisionReason::Other)
                .await
                .unwrap_err()
                .code(),
            "outOfScope"
        );
        assert_eq!(
            svc.request_update(&reviewer, case_id, 1, DecisionReason::Other)
                .await
                .unwrap_err()
                .code(),
            "outOfScope"
        );
    }

    db.finish().await;
}

// ── 13. Not found: Action on non-existent case ───────────────────────────────

#[tokio::test]
async fn acting_on_non_existent_case_returns_not_found() {
    let db = Db::start().await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();
    let missing_id = Uuid::new_v4();

    let err = svc
        .start_review(&reviewer, missing_id, 1)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "notFound");
    assert_eq!(err, ReviewError::NotFound);

    db.finish().await;
}

// ── 14. Stale version: Expected version does not match ───────────────────────

#[tokio::test]
async fn stale_version_when_expected_version_does_not_match() {
    let db = Db::start().await;
    let customer = db.customer(ORG_1).await;
    let case_id = db
        .insert_case(ORG_1, &customer.id, "SUBMITTED", None, 1)
        .await;
    let reviewer = db.org_reviewer(ORG_1).await;
    let svc = db.service();

    // Case is at version 1, reviewer sends expected_version = 99
    let err = svc.start_review(&reviewer, case_id, 99).await.unwrap_err();
    assert_eq!(err.code(), "staleVersion");
    assert_eq!(err, ReviewError::StaleVersion);

    // Case is untouched
    let version = db
        .count(&format!(
            "SELECT version::int8 FROM verification_cases WHERE id = '{case_id}'"
        ))
        .await;
    assert_eq!(version, 1);

    db.finish().await;
}
