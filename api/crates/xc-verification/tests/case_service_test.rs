//! VF-3 #1389 — the customer side of a verification case: `CaseService` (open, add/remove a document, submit),
//! `CaseMachine`'s customer steps, `CaseStatus`, `DecisionReason` (design `docs/design/blueprint/verification.md`
//! V2, V3, V6, V11, V14, V16, V17, §4.2, §4.3, §12 S-5; §9.1 row VF-3; ticket #1389 §4 item 1 and §9 edge cases).
//! Written before the implementation by the test writer (claude/dell-claude-4, on the BE lane controller
//! claude/ad05bf8d's order, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The database comes from every migration in `api/migrations/` (VF-1's M1 `<version>_verification.sql` included,
//! which seeds policy version 1 for every organisation existing at M1). Each test works in a schema of its own.
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.
//!
//! The document store is an in-memory `DocumentStore` (VF-2's trait, `xc-kernel/src/document_store.rs`) defined
//! here, so the tests can count objects and read what was stored.
//!
//! Not pinned here, asked on #1389 (the tests accept either answer): which refusal `submit` gives on a state it
//! cannot leave; whether `open` on a `VERIFIED` customer starts a new case; which of `file_service.rs`'s two type
//! sentences refuses content that is none of JPEG, PNG or PDF.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures::future::BoxFuture;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;
use xc_kernel::pii::identity_type::IdentityType;
use xc_verification::application::case_service::CaseService;
use xc_verification::domain::case::case_actor::CaseActor;
use xc_verification::domain::case::case_error::CaseError;
use xc_verification::domain::case::case_status::CaseStatus;
use xc_verification::domain::case::customer_actor::CustomerActor;
use xc_verification::domain::case::decision_reason::DecisionReason;
use xc_verification::domain::case::provider::Provider;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

/// The organisation OR-1's migration creates; M1 seeds its policy version 1.
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
/// `MAX_UPLOAD_BYTES`, `api/src/infrastructure/files/file_service.rs:12`.
const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;
/// `file_service.rs:17` `TOO_LARGE`.
const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
/// `file_service.rs:16` `UNSUPPORTED` and `:18` `NOT_THAT_KIND`.
const TYPE_SENTENCES: [&str; 2] = ["ชนิดไฟล์ไม่รองรับ", "ชนิดไฟล์ไม่ตรงกับเนื้อหา"];
/// A text inside every uploaded test file; no audit row may carry it (V6).
const MARKER: &[u8] = b"VF3-SECRET-BYTES-7c1e";
/// The key id the in-memory store answers for every put.
const STORE_KEY_ID: &str = "test-key-1";
const MUST_FINISH: Duration = Duration::from_secs(60);
/// The statuses a customer command must never produce (ticket §4 item 1, design §4.3).
const REVIEWER_STATES: [CaseStatus; 4] = [
    CaseStatus::UnderReview,
    CaseStatus::Verified,
    CaseStatus::Rejected,
    CaseStatus::Revoked,
];

static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ── files ───────────────────────────────────────────────────────────────────

fn png() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(MARKER);
    b.extend_from_slice(&unique("png").into_bytes());
    b
}

fn jpeg() -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0];
    b.extend_from_slice(MARKER);
    b
}

fn pdf() -> Vec<u8> {
    let mut b = b"%PDF-1.7\n".to_vec();
    b.extend_from_slice(MARKER);
    b
}

fn webp() -> Vec<u8> {
    let mut b = b"RIFF\x10\x00\x00\x00WEBPVP8 ".to_vec();
    b.extend_from_slice(MARKER);
    b
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

// ── the in-memory DocumentStore ─────────────────────────────────────────────

#[derive(Default)]
struct MemStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    puts: AtomicUsize,
}

impl MemStore {
    fn objects(&self) -> HashMap<String, Vec<u8>> {
        self.objects.lock().expect("not poisoned").clone()
    }
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
            Ok(STORE_KEY_ID.to_owned())
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

// ── the database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    schema: String,
    store: Arc<MemStore>,
}

/// A customer with their user (`customers.user_id`, the person the upload is recorded against).
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
        let schema = unique("vf3");
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
        let db = Self {
            pool,
            schema,
            store: Arc::new(MemStore::default()),
        };
        assert_eq!(
            db.count("SELECT count(*) FROM verification_policies WHERE version = 1")
                .await,
            db.count("SELECT count(*) FROM organizations").await,
            "VF-1's M1 seeded policy version 1 for every organisation"
        );
        db
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

    /// An organisation created after M1: it has no policy version.
    async fn organisation(&self) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('{id}', '{}', 'Asia/Bangkok', now())",
            unique("org")
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

    async fn branch(&self, org: &str) -> BranchId {
        let id = unique("B").replace('_', "-");
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{id}', '{id}', '{org}'::uuid)"
        ))
        .await;
        BranchId::new(id)
    }

    /// A VERIFICATION_REVIEWER scoped to a branch of `org` (V17).
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

    /// Publish a policy version of `org` (version 1 may have no publisher; later ones need one).
    async fn policy(&self, org: &str, version: i32, types: &[&str], required: i32) {
        let publisher = self.user(org).await;
        let types = types
            .iter()
            .map(|t| format!("'{t}'"))
            .collect::<Vec<_>>()
            .join(",");
        self.exec(&format!(
            "INSERT INTO verification_policies (organization_id, version, accepted_document_types, \
             documents_required, branch_review_allowed, published_by, published_at, created_at) \
             VALUES ('{org}'::uuid, {version}, ARRAY[{types}], {required}, true, '{publisher}', now(), now())"
        ))
        .await;
    }

    async fn status(&self, case: Uuid) -> CaseStatus {
        let text = self
            .text(&format!(
                "SELECT status FROM verification_cases WHERE id = '{case}'"
            ))
            .await
            .expect("the case row");
        CaseStatus::parse(&text).unwrap_or_else(|| panic!("a known status: {text}"))
    }

    /// Every case of the customer: (id, status).
    async fn cases_of(&self, customer: &Customer) -> Vec<(Uuid, String)> {
        sqlx::query_as(
            "SELECT id, status FROM verification_cases WHERE customer_id = $1 ORDER BY created_at, id",
        )
        .bind(&customer.id)
        .fetch_all(&self.pool)
        .await
        .expect("the customer's cases")
    }

    /// Force a case into `status` with the columns its CHECKs need (reviewer, decided_at, a reason).
    async fn force(&self, case: Uuid, status: CaseStatus, reviewer: Uuid) {
        let decided = !matches!(
            status,
            CaseStatus::Draft | CaseStatus::Submitted | CaseStatus::UnderReview
        );
        let reasoned = matches!(
            status,
            CaseStatus::Rejected | CaseStatus::NeedsUpdate | CaseStatus::Revoked
        );
        let (reviewed_by, decided_at) = if decided {
            (format!("'{reviewer}'::uuid"), "now()")
        } else {
            ("NULL".to_owned(), "NULL")
        };
        let reason = if reasoned { "'OTHER'" } else { "NULL" };
        let submitted = if status == CaseStatus::Draft {
            "NULL"
        } else {
            "now()"
        };
        self.exec(&format!(
            "UPDATE verification_cases SET status = '{}', submitted_at = {submitted}, reviewed_by = {reviewed_by}, \
             decided_at = {decided_at}, decision_reason_code = {reason} WHERE id = '{case}'",
            status.as_str()
        ))
        .await;
    }

    async fn documents(&self, case: Uuid) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE case_id = '{case}'"
        ))
        .await
    }

    async fn audit_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM audit_events").await
    }

    /// Everything the audit rows hold, as one text.
    async fn audit_text(&self) -> String {
        self.text(
            "SELECT coalesce(string_agg(concat_ws(' | ', actor_id, actor_role, scope, action, object_ref, \
             branch_id, before_redacted::text, after_redacted::text), E'\\n'), '') FROM audit_events",
        )
        .await
        .unwrap_or_default()
    }
}

// ── the policy ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_organisation_with_no_policy_version_gets_no_verification_policy_and_no_case() {
    let db = Db::start().await;
    let org = db.organisation().await;
    let c = db.customer(&org.to_string()).await;
    let err = db
        .service()
        .open(&c.actor(), &c.id)
        .await
        .expect_err("no policy version");
    assert!(matches!(err, CaseError::NoVerificationPolicy), "{err:?}");
    assert_eq!(err.code(), "noVerificationPolicy");
    assert!(db.cases_of(&c).await.is_empty(), "no case written");
    assert_eq!(db.audit_rows().await, 0, "a refusal writes no audit row");
    db.finish().await;
}

#[tokio::test]
async fn under_version_1_a_case_with_one_passport_can_be_submitted() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    assert_eq!(case.status, CaseStatus::Draft);
    assert_eq!(case.policy_version, 1);
    assert_eq!(case.provider, Provider::Staff);
    assert_eq!(case.customer_id, c.id);
    assert_eq!(case.organization_id.to_string(), ORG_1);
    assert_eq!(
        case.branch_id, None,
        "an app case goes to the organisation's queue (V17)"
    );
    svc.add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k1")
        .await
        .expect("a passport");
    let submitted = svc.submit(&c.actor(), case.id).await.expect("submit");
    assert_eq!(submitted.status, CaseStatus::Submitted);
    assert!(submitted.submitted_at.is_some());
    assert_eq!(db.status(case.id).await, CaseStatus::Submitted);
    db.finish().await;
}

#[tokio::test]
async fn under_version_1_a_case_with_one_national_id_can_be_submitted() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let doc = svc
        .add_document(&c.actor(), case.id, IdentityType::NationalId, &jpeg(), "k1")
        .await
        .expect("a national ID card");
    assert_eq!(doc.document_type, IdentityType::NationalId);
    assert_eq!(
        db.text(&format!(
            "SELECT document_type FROM verification_documents WHERE id = '{}'",
            doc.id
        ))
        .await
        .as_deref(),
        Some("NATIONAL_ID")
    );
    let submitted = svc.submit(&c.actor(), case.id).await.expect("submit");
    assert_eq!(submitted.status, CaseStatus::Submitted);
    db.finish().await;
}

#[tokio::test]
async fn phone_and_email_documents_are_refused_and_write_nothing() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let audit = db.audit_rows().await;
    for t in [IdentityType::Phone, IdentityType::Email] {
        let err = svc
            .add_document(&c.actor(), case.id, t, &png(), &unique("k"))
            .await
            .expect_err("not a document type of version 1");
        assert!(
            matches!(err, CaseError::DocumentTypeNotAccepted),
            "{t:?}: {err:?}"
        );
        assert_eq!(err.code(), "documentTypeNotAccepted");
    }
    assert_eq!(db.documents(case.id).await, 0);
    assert!(db.store.objects().is_empty(), "no object stored");
    assert_eq!(
        db.audit_rows().await,
        audit,
        "a refusal writes no audit row"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_type_the_case_policy_version_does_not_accept_is_refused() {
    // The organisation publishes version 2 (passport only, two documents); a case opened now records version 2.
    let db = Db::start().await;
    let org = db.organisation().await.to_string();
    db.exec(&format!(
        "INSERT INTO verification_policies (organization_id, version, accepted_document_types, documents_required, \
         branch_review_allowed, published_at, created_at) VALUES ('{org}'::uuid, 1, ARRAY['PASSPORT','NATIONAL_ID'], 1, true, now(), now())"
    ))
    .await;
    db.policy(&org, 2, &["PASSPORT"], 2).await;
    let c = db.customer(&org).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    assert_eq!(case.policy_version, 2, "the newest published version");
    let err = svc
        .add_document(&c.actor(), case.id, IdentityType::NationalId, &png(), "k1")
        .await
        .expect_err("version 2 accepts passports only");
    assert!(matches!(err, CaseError::DocumentTypeNotAccepted), "{err:?}");
    svc.add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k2")
        .await
        .expect("a passport");
    let err = svc
        .submit(&c.actor(), case.id)
        .await
        .expect_err("version 2 needs two documents");
    assert!(matches!(err, CaseError::DocumentsMissing), "{err:?}");
    svc.add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k3")
        .await
        .expect("a second passport scan");
    let s = svc
        .submit(&c.actor(), case.id)
        .await
        .expect("two documents");
    assert_eq!(s.status, CaseStatus::Submitted);
    db.finish().await;
}

// ── open ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn opening_twice_returns_one_case() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let first = svc.open(&c.actor(), &c.id).await.expect("open");
    let second = svc.open(&c.actor(), &c.id).await.expect("open again");
    assert_eq!(first.id, second.id);
    assert_eq!(db.cases_of(&c).await.len(), 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_opens_for_one_customer_give_one_case() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    for _ in 0..5 {
        let (a, b) = (db.service(), db.service());
        let (actor_a, actor_b) = (c.actor(), c.actor());
        let (id_a, id_b) = (c.id.clone(), c.id.clone());
        let (x, y) = tokio::join!(
            tokio::spawn(async move { a.open(&actor_a, &id_a).await.map(|k| k.id) }),
            tokio::spawn(async move { b.open(&actor_b, &id_b).await.map(|k| k.id) }),
        );
        let x = x.expect("ran").expect("open");
        let y = y.expect("ran").expect("open");
        assert_eq!(x, y, "both opens answer the same case");
    }
    assert_eq!(db.cases_of(&c).await.len(), 1);
    db.finish().await;
}

// ── submit and the documents ────────────────────────────────────────────────

#[tokio::test]
async fn submit_with_no_document_is_documents_missing_and_stays_draft() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let audit = db.audit_rows().await;
    let err = svc
        .submit(&c.actor(), case.id)
        .await
        .expect_err("no document");
    assert!(matches!(err, CaseError::DocumentsMissing), "{err:?}");
    assert_eq!(err.code(), "documentsMissing");
    assert_eq!(db.status(case.id).await, CaseStatus::Draft);
    assert_eq!(
        db.audit_rows().await,
        audit,
        "a refusal writes no audit row"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_removed_document_no_longer_counts_and_keeps_its_row_without_a_ref() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let doc = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k1")
        .await
        .expect("a passport");
    svc.remove_document(&c.actor(), doc.id)
        .await
        .expect("remove while DRAFT");
    assert_eq!(db.documents(case.id).await, 1, "the row is kept");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE id = '{}' \
             AND removed_at IS NOT NULL AND encrypted_object_ref IS NULL",
            doc.id
        ))
        .await,
        1,
        "removed_at set, ref cleared"
    );
    let err = svc
        .submit(&c.actor(), case.id)
        .await
        .expect_err("the only document was removed");
    assert!(matches!(err, CaseError::DocumentsMissing), "{err:?}");
    db.finish().await;
}

#[tokio::test]
async fn after_submit_add_and_remove_are_case_locked() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let doc = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k1")
        .await
        .expect("a passport");
    svc.submit(&c.actor(), case.id).await.expect("submit");
    let audit = db.audit_rows().await;
    let err = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k2")
        .await
        .expect_err("frozen after submit");
    assert!(matches!(err, CaseError::CaseLocked), "{err:?}");
    assert_eq!(err.code(), "caseLocked");
    let err = svc
        .remove_document(&c.actor(), doc.id)
        .await
        .expect_err("frozen after submit");
    assert!(matches!(err, CaseError::CaseLocked), "{err:?}");
    assert_eq!(db.documents(case.id).await, 1);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_documents WHERE id = '{}' AND removed_at IS NULL",
            doc.id
        ))
        .await,
        1,
        "the document is not removed"
    );
    assert_eq!(db.store.objects().len(), 1, "no object added or deleted");
    assert_eq!(db.status(case.id).await, CaseStatus::Submitted);
    assert_eq!(db.audit_rows().await, audit, "refusals write no audit row");
    db.finish().await;
}

#[tokio::test]
async fn needs_update_allows_add_and_remove_and_resubmits_to_submitted() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let reviewer = db.user(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let old = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k1")
        .await
        .expect("a passport");
    svc.submit(&c.actor(), case.id).await.expect("submit");
    db.force(case.id, CaseStatus::NeedsUpdate, reviewer).await;
    svc.add_document(&c.actor(), case.id, IdentityType::NationalId, &pdf(), "k2")
        .await
        .expect("add while NEEDS_UPDATE");
    svc.remove_document(&c.actor(), old.id)
        .await
        .expect("remove while NEEDS_UPDATE");
    let s = svc.submit(&c.actor(), case.id).await.expect("resubmit");
    assert_eq!(s.status, CaseStatus::Submitted);
    assert_eq!(db.status(case.id).await, CaseStatus::Submitted);
    db.finish().await;
}

// ── no customer command reaches a reviewer state ────────────────────────────

#[derive(Clone, Copy, Debug)]
enum Command {
    Open,
    Add,
    Remove,
    Submit,
}

#[tokio::test]
async fn no_customer_command_reaches_under_review_verified_rejected_or_revoked_from_any_state() {
    // Each customer command on each of the 8 states, each pair on a fresh customer whose case holds one passport.
    let db = Db::start().await;
    let reviewer = db.user(ORG_1).await;
    let svc = db.service();
    for from in CaseStatus::ALL {
        for command in [
            Command::Open,
            Command::Add,
            Command::Remove,
            Command::Submit,
        ] {
            let what = format!("{command:?} on {}", from.as_str());
            let c = db.customer(ORG_1).await;
            let case = svc.open(&c.actor(), &c.id).await.expect("open");
            let doc = svc
                .add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k1")
                .await
                .expect("a passport");
            if from != CaseStatus::Draft {
                db.force(case.id, from, reviewer).await;
            }
            let editable = matches!(from, CaseStatus::Draft | CaseStatus::NeedsUpdate);
            let out = match command {
                Command::Open => svc.open(&c.actor(), &c.id).await.map(|_| ()),
                Command::Add => svc
                    .add_document(&c.actor(), case.id, IdentityType::Passport, &png(), "k2")
                    .await
                    .map(|_| ()),
                Command::Remove => svc.remove_document(&c.actor(), doc.id).await,
                Command::Submit => svc.submit(&c.actor(), case.id).await.map(|_| ()),
            };
            let after = db.status(case.id).await;
            match command {
                Command::Submit if editable => {
                    assert!(out.is_ok(), "{what}: {out:?}");
                    assert_eq!(after, CaseStatus::Submitted, "{what}");
                }
                Command::Submit => {
                    assert!(out.is_err(), "{what} is refused");
                    assert_eq!(after, from, "{what} changes nothing");
                }
                Command::Add | Command::Remove if !editable => {
                    let err = out.expect_err(&what);
                    assert!(matches!(err, CaseError::CaseLocked), "{what}: {err:?}");
                    assert_eq!(after, from, "{what}");
                }
                _ => {
                    assert!(out.is_ok(), "{what}: {out:?}");
                    assert_eq!(after, from, "{what}");
                }
            }
            for (id, status) in db.cases_of(&c).await {
                let status = CaseStatus::parse(&status).expect("a known status");
                if id == case.id {
                    assert!(
                        !REVIEWER_STATES.contains(&status) || status == from,
                        "{what}: reached {}",
                        status.as_str()
                    );
                } else {
                    assert_eq!(status, CaseStatus::Draft, "{what}: a new case starts DRAFT");
                }
            }
            if matches!(command, Command::Open) {
                let n = db.cases_of(&c).await.len();
                if from.is_open() {
                    assert_eq!(n, 1, "{what}: the open case is returned");
                }
                if from.is_final() {
                    assert_eq!(
                        n, 2,
                        "{what}: an ended case is history; a new case opens (V2)"
                    );
                }
            }
        }
    }
    db.finish().await;
}

#[tokio::test]
async fn a_branch_reviewer_opens_adds_and_submits_and_reaches_no_reviewer_state() {
    // V17: a VERIFICATION_REVIEWER scoped to a branch, for a customer present at the desk.
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let r = db.branch_reviewer(ORG_1).await;
    let actor = CaseActor::Reviewer(r.clone());
    let svc = db.service();
    let case = svc
        .open(&actor, &c.id)
        .await
        .expect("open for the customer");
    assert_eq!(case.status, CaseStatus::Draft);
    assert_eq!(case.customer_id, c.id);
    let doc = svc
        .add_document(&actor, case.id, IdentityType::Passport, &png(), "k1")
        .await
        .expect("attach the scan");
    assert_eq!(doc.uploaded_by, r.user_id, "the reviewer is the uploader");
    let s = svc.submit(&actor, case.id).await.expect("submit");
    assert_eq!(s.status, CaseStatus::Submitted, "submitted, not decided");
    assert!(s.reviewed_by.is_none() && s.decided_at.is_none());
    assert_eq!(db.status(case.id).await, CaseStatus::Submitted);
    db.finish().await;
}

// ── scope ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn another_customer_or_another_organisation_is_out_of_scope_and_changes_nothing() {
    let db = Db::start().await;
    let owner = db.customer(ORG_1).await;
    let other = db.customer(ORG_1).await;
    let foreign_org = db.organisation().await.to_string();
    let foreign = db.branch_reviewer(&foreign_org).await;
    let svc = db.service();
    let case = svc.open(&owner.actor(), &owner.id).await.expect("open");
    let doc = svc
        .add_document(
            &owner.actor(),
            case.id,
            IdentityType::Passport,
            &png(),
            "k1",
        )
        .await
        .expect("a passport");
    let audit = db.audit_rows().await;
    for (who, actor) in [
        ("another customer", other.actor()),
        (
            "a reviewer of another organisation",
            CaseActor::Reviewer(foreign.clone()),
        ),
    ] {
        let err = svc.open(&actor, &owner.id).await.expect_err(who);
        assert!(
            matches!(err, CaseError::OutOfScope),
            "open by {who}: {err:?}"
        );
        assert_eq!(err.code(), "outOfScope");
        let err = svc
            .add_document(&actor, case.id, IdentityType::Passport, &png(), "k9")
            .await
            .expect_err(who);
        assert!(
            matches!(err, CaseError::OutOfScope),
            "add by {who}: {err:?}"
        );
        let err = svc.remove_document(&actor, doc.id).await.expect_err(who);
        assert!(
            matches!(err, CaseError::OutOfScope),
            "remove by {who}: {err:?}"
        );
        let err = svc.submit(&actor, case.id).await.expect_err(who);
        assert!(
            matches!(err, CaseError::OutOfScope),
            "submit by {who}: {err:?}"
        );
    }
    assert_eq!(db.status(case.id).await, CaseStatus::Draft);
    assert_eq!(db.documents(case.id).await, 1);
    assert_eq!(db.store.objects().len(), 1);
    assert!(db.cases_of(&other).await.is_empty());
    assert_eq!(db.audit_rows().await, audit, "refusals write no audit row");
    db.finish().await;
}

// ── the upload ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_stored_checksum_is_the_sha256_of_the_upload_and_the_bytes_go_to_the_store() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let bytes = png();
    let doc = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &bytes, "k1")
        .await
        .expect("a passport");
    assert_eq!(doc.checksum, sha256(&bytes), "the answer's checksum");
    let (checksum, size, mime, key_id, object_ref, uploaded_by, class): (
        Vec<u8>,
        i32,
        String,
        Option<String>,
        Option<String>,
        Uuid,
        String,
    ) = sqlx::query_as(
        "SELECT checksum, size_bytes, mime, key_id, encrypted_object_ref, uploaded_by, access_class \
         FROM verification_documents WHERE id = $1",
    )
    .bind(doc.id)
    .fetch_one(&db.pool)
    .await
    .expect("the document row");
    assert_eq!(checksum, sha256(&bytes), "the stored checksum");
    assert_eq!(size as usize, bytes.len());
    assert_eq!(mime, "image/png");
    assert_eq!(
        key_id.as_deref(),
        Some(STORE_KEY_ID),
        "the key id the store answered (S-2)"
    );
    assert_eq!(uploaded_by, c.user, "the customer's user is the uploader");
    assert_eq!(class, "IDENTITY_DOCUMENT");
    let object_ref = object_ref.expect("a store ref");
    assert_eq!(
        doc.encrypted_object_ref.as_deref(),
        Some(object_ref.as_str())
    );
    assert_eq!(
        db.store.objects().get(&object_ref),
        Some(&bytes),
        "the store holds the upload under the row's ref"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_retried_upload_with_the_same_key_writes_one_row_and_one_object() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let bytes = png();
    let first = svc
        .add_document(
            &c.actor(),
            case.id,
            IdentityType::Passport,
            &bytes,
            "retry-key",
        )
        .await
        .expect("the upload");
    let audit = db.audit_rows().await;
    let again = svc
        .add_document(
            &c.actor(),
            case.id,
            IdentityType::Passport,
            &bytes,
            "retry-key",
        )
        .await
        .expect("the retry");
    assert_eq!(again.id, first.id, "the retry answers the same document");
    assert_eq!(db.documents(case.id).await, 1, "one row");
    assert_eq!(db.store.objects().len(), 1, "one object");
    assert_eq!(db.audit_rows().await, audit, "a replay is not a new write");
    db.finish().await;
}

#[tokio::test]
async fn the_same_key_with_another_body_is_an_idempotency_conflict() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let bytes = png();
    let first = svc
        .add_document(
            &c.actor(),
            case.id,
            IdentityType::Passport,
            &bytes,
            "same-key",
        )
        .await
        .expect("the upload");
    let err = svc
        .add_document(
            &c.actor(),
            case.id,
            IdentityType::Passport,
            &png(),
            "same-key",
        )
        .await
        .expect_err("another body under the same key");
    assert!(matches!(err, CaseError::IdempotencyConflict), "{err:?}");
    assert_eq!(err.code(), "idempotencyConflict");
    assert_eq!(db.documents(case.id).await, 1);
    let objects = db.store.objects();
    assert_eq!(objects.len(), 1);
    assert_eq!(
        objects.get(first.encrypted_object_ref.as_deref().expect("a ref")),
        Some(&bytes),
        "the first upload's object is unchanged"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_content_decides_the_type_jpeg_png_and_pdf_only() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    for (bytes, mime) in [
        (jpeg(), "image/jpeg"),
        (png(), "image/png"),
        (pdf(), "application/pdf"),
    ] {
        let doc = svc
            .add_document(
                &c.actor(),
                case.id,
                IdentityType::Passport,
                &bytes,
                &unique("k"),
            )
            .await
            .unwrap_or_else(|e| panic!("{mime}: {e:?}"));
        assert_eq!(doc.mime, mime);
    }
    let kept = db.documents(case.id).await;
    let objects = db.store.objects().len();
    for (what, bytes) in [
        ("webp", webp()),
        ("plain text", b"just text, no magic".to_vec()),
        ("a PNG signature cut short", vec![0x89, 0x50, 0x4E, 0x47]),
    ] {
        let err = svc
            .add_document(
                &c.actor(),
                case.id,
                IdentityType::Passport,
                &bytes,
                &unique("k"),
            )
            .await
            .expect_err(what);
        match err {
            CaseError::File(sentence) => assert!(
                TYPE_SENTENCES.contains(&sentence),
                "{what}: one of file_service.rs's type sentences, got {sentence}"
            ),
            other => panic!("{what}: a file refusal, got {other:?}"),
        }
    }
    assert_eq!(
        db.documents(case.id).await,
        kept,
        "refused uploads write no row"
    );
    assert_eq!(db.store.objects().len(), objects, "and no object");
    db.finish().await;
}

#[tokio::test]
async fn fifteen_megabytes_is_accepted_and_one_byte_more_is_refused_with_the_size_sentence() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    let svc = db.service();
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    let mut bytes = png();
    bytes.resize(MAX_UPLOAD_BYTES, 0);
    let doc = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &bytes, "exact")
        .await
        .expect("exactly 15 MB");
    assert_eq!(doc.size_bytes as usize, MAX_UPLOAD_BYTES);
    bytes.push(0);
    let err = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &bytes, "over")
        .await
        .expect_err("15 MB + 1 byte");
    match err {
        CaseError::File(sentence) => assert_eq!(sentence, TOO_LARGE),
        other => panic!("the size sentence, got {other:?}"),
    }
    assert_eq!(db.documents(case.id).await, 1);
    assert_eq!(db.store.objects().len(), 1);
    db.finish().await;
}

// ── the audit ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn every_customer_write_writes_one_audit_row_with_no_bytes_and_no_identity_value() {
    let db = Db::start().await;
    let c = db.customer(ORG_1).await;
    db.exec(&format!(
        "UPDATE customers SET display_name = 'Somchai IDV-1234567890123' WHERE id = '{}'",
        c.id
    ))
    .await;
    let svc = db.service();
    let start = db.audit_rows().await;
    let case = svc.open(&c.actor(), &c.id).await.expect("open");
    assert_eq!(db.audit_rows().await, start + 1, "open writes one row");
    let bytes = png();
    let doc = svc
        .add_document(&c.actor(), case.id, IdentityType::Passport, &bytes, "k1")
        .await
        .expect("add");
    assert_eq!(db.audit_rows().await, start + 2, "add writes one row");
    svc.remove_document(&c.actor(), doc.id)
        .await
        .expect("remove");
    assert_eq!(db.audit_rows().await, start + 3, "remove writes one row");
    svc.add_document(&c.actor(), case.id, IdentityType::NationalId, &jpeg(), "k2")
        .await
        .expect("add");
    svc.submit(&c.actor(), case.id).await.expect("submit");
    assert_eq!(
        db.audit_rows().await,
        start + 5,
        "add and submit write one row each"
    );
    let text = db.audit_text().await;
    let marker = String::from_utf8_lossy(MARKER).into_owned();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert!(!text.contains(&marker), "no upload bytes: {text}");
    assert!(
        !text.contains(&b64[..24]),
        "no base64 of the upload: {text}"
    );
    assert!(!text.contains(&hex[..32]), "no hex of the upload: {text}");
    assert!(!text.contains("1234567890123"), "no identity value: {text}");
    assert!(
        text.contains(&case.id.to_string()),
        "the rows name the case: {text}"
    );
    db.finish().await;
}

// ── the domain values ───────────────────────────────────────────────────────

#[test]
fn the_eight_states_round_trip_and_say_which_are_open_and_which_final() {
    let open = [
        CaseStatus::Draft,
        CaseStatus::Submitted,
        CaseStatus::UnderReview,
        CaseStatus::NeedsUpdate,
    ];
    let fin = [
        CaseStatus::Rejected,
        CaseStatus::Expired,
        CaseStatus::Revoked,
    ];
    let texts = [
        "DRAFT",
        "SUBMITTED",
        "UNDER_REVIEW",
        "VERIFIED",
        "REJECTED",
        "NEEDS_UPDATE",
        "EXPIRED",
        "REVOKED",
    ];
    for (s, t) in CaseStatus::ALL.iter().zip(texts) {
        assert_eq!(s.as_str(), t);
        assert_eq!(CaseStatus::parse(t), Some(*s));
        assert_eq!(s.is_open(), open.contains(s), "{t} is_open");
        assert_eq!(s.is_final(), fin.contains(s), "{t} is_final");
    }
    assert!(!CaseStatus::Verified.is_open() && !CaseStatus::Verified.is_final());
    assert_eq!(CaseStatus::parse("draft"), None);
    assert_eq!(CaseStatus::parse("APPROVED"), None);
    assert_eq!(Provider::Staff.as_str(), "STAFF");
}

#[test]
fn a_decision_reason_is_one_of_the_six_codes_the_owner_decided_and_nothing_else() {
    // S-5, owner 2026-10-12 (design §12): no free text.
    let codes = [
        "DOCUMENT_UNREADABLE",
        "DOCUMENT_EXPIRED",
        "DETAILS_MISMATCH",
        "DOCUMENT_NOT_ACCEPTED",
        "SUSPECTED_FRAUD",
        "OTHER",
    ];
    for (r, code) in DecisionReason::ALL.iter().zip(codes) {
        assert_eq!(r.as_str(), code);
        assert_eq!(DecisionReason::parse(code), Some(*r));
    }
    for bad in [
        "",
        "other",
        "OTHER ",
        " OTHER",
        "document_unreadable",
        "BLURRY",
        "EXPIRED",
        "the photo is blurry",
    ] {
        assert_eq!(DecisionReason::parse(bad), None, "{bad:?} is not a code");
    }
}
