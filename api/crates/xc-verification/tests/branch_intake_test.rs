//! VF-12 (#1398) — at-branch intake of a walk-in customer: `BranchIntake` (open, store document, submit in one transaction),
//! scoped to a branch VERIFICATION_REVIEWER. Design `docs/design/blueprint/verification.md` V17, §4.3, §4.5, §5, §8, §9.1 row VF-12.
//!
//! Written before the implementation by the test writer (gemini/warut-5, on the BE lane controller
//! claude/ad05bf8d's order, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.
//! Each test works in a schema of its own.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEntry, AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;
use xc_kernel::pii::identity_type::IdentityType;
use xc_verification::application::branch_intake::{
    BranchDocument, BranchIntake, BranchIntakeError,
};
use xc_verification::domain::case::case_status::CaseStatus;
use xc_verification::domain::case::provider::Provider;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::domain::document::access_class::AccessClass;
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

/// The organisation OR-1's migration creates; M1 seeds its policy version 1.
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
/// `MAX_UPLOAD_BYTES`, `api/src/infrastructure/files/file_service.rs:12`.
const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;
/// `file_service.rs:17` `TOO_LARGE`.
const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
/// A text inside every uploaded test file; no audit row may carry it (V6).
const MARKER: &[u8] = b"VF12-SECRET-BYTES-9d4f";
/// The key id the in-memory store answers for every put.
const STORE_KEY_ID: &str = "test-vault-key-1";
const MUST_FINISH: Duration = Duration::from_secs(60);

static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ── test file payloads ──────────────────────────────────────────────────────

fn png() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(MARKER);
    b.extend_from_slice(&unique("png").into_bytes());
    b
}

fn jpeg() -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0];
    b.extend_from_slice(MARKER);
    b.extend_from_slice(&unique("jpeg").into_bytes());
    b
}

fn pdf() -> Vec<u8> {
    let mut b = b"%PDF-1.7\n".to_vec();
    b.extend_from_slice(MARKER);
    b.extend_from_slice(&unique("pdf").into_bytes());
    b
}

fn invalid_bytes() -> Vec<u8> {
    let mut b = b"NOT_A_VALID_IMAGE_OR_PDF".to_vec();
    b.extend_from_slice(MARKER);
    b
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

// ── in-memory DocumentStore ─────────────────────────────────────────────────

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

// ── failing AuditLog mock for rollback test ─────────────────────────────────

struct FailingAuditLog {
    inner: Arc<dyn AuditLog>,
    should_fail: Arc<AtomicBool>,
}

impl AuditLog for FailingAuditLog {
    fn append<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a AuditEvent,
    ) -> BoxFuture<'a, AppResult<i64>> {
        if self.should_fail.load(Ordering::Relaxed) {
            Box::pin(async move { Err(AppError::Internal("simulated audit failure".to_string())) })
        } else {
            self.inner.append(tx, event)
        }
    }

    fn page<'a>(
        &'a self,
        after_id: Option<i64>,
        limit: i64,
    ) -> BoxFuture<'a, AppResult<Vec<AuditEntry>>> {
        self.inner.page(after_id, limit)
    }
}

// ── database fixture ────────────────────────────────────────────────────────

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
        let schema = unique("vf12");
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

    fn intake_with_audit_log(&self, audit_log: Arc<dyn AuditLog>) -> BranchIntake {
        BranchIntake::new(
            self.pool.clone(),
            Arc::new(PgCaseRepository::new(self.pool.clone())),
            self.store.clone(),
            audit_log,
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

    async fn audit_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM audit_events").await
    }

    async fn cases_count(&self) -> i64 {
        self.count("SELECT count(*) FROM verification_cases").await
    }

    async fn documents_count(&self) -> i64 {
        self.count("SELECT count(*) FROM verification_documents")
            .await
    }

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

    async fn org_scoped_reviewer(&self, org: &str) -> ReviewerActor {
        let user_id = self.user(org).await;
        ReviewerActor {
            user_id,
            scope: ReviewerScope::Organization {
                organization_id: Uuid::parse_str(org).expect("uuid"),
            },
        }
    }

    async fn set_policy_branch_review_allowed(&self, org: &str, version: i32, allowed: bool) {
        self.exec(&format!(
            "UPDATE verification_policies SET branch_review_allowed = {allowed} \
             WHERE organization_id = '{org}'::uuid AND version = {version}"
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

// ── tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn branch_review_allowed_false_is_refused_with_branch_review_not_allowed() {
    // Design §4.3, #1398 done criteria 1: with branch_review_allowed = false → branchReviewNotAllowed.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    db.set_policy_branch_review_allowed(ORG_1, 1, false).await;

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k1");
    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("branch_review_allowed = false must refuse");

    assert!(matches!(err, BranchIntakeError::BranchReviewNotAllowed));
    assert_eq!(err.code(), "branchReviewNotAllowed");
    assert_eq!(db.cases_count().await, 0, "no case row written");
    assert_eq!(db.documents_count().await, 0, "no document row written");
    assert_eq!(db.audit_rows().await, 0, "no audit row written on refusal");
    assert_eq!(db.store.objects().len(), 0, "no vault object kept");
    db.finish().await;
}

#[tokio::test]
async fn organisation_scoped_reviewer_is_refused_out_of_scope() {
    // Design §4.3, V17, #1398 done criteria 1: an organisation-scoped reviewer is refused outOfScope.
    let db = Db::start().await;
    let org_reviewer = db.org_scoped_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k1");
    let err = intake
        .open(&org_reviewer, &customer.id, doc)
        .await
        .expect_err("organisation-scoped reviewer cannot perform branch intake");

    assert!(matches!(err, BranchIntakeError::OutOfScope));
    assert_eq!(err.code(), "outOfScope");
    assert_eq!(db.cases_count().await, 0);
    assert_eq!(db.audit_rows().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn customer_of_another_organisation_is_refused_out_of_scope() {
    // Design §4.3, #1398 §9 edge cases: customer of another organisation → outOfScope.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;

    let org_2 = db.organisation().await.to_string();
    let foreign_customer = db.customer(&org_2).await;

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k1");
    let err = intake
        .open(&reviewer, &foreign_customer.id, doc)
        .await
        .expect_err("customer of another organisation must be refused");

    assert!(matches!(err, BranchIntakeError::OutOfScope));
    assert_eq!(err.code(), "outOfScope");
    assert_eq!(db.cases_count().await, 0);
    assert_eq!(db.audit_rows().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn reviewer_linked_to_customer_is_refused_out_of_scope() {
    // Design V3, #1398 done criteria 1: a reviewer linked to that customer → outOfScope.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;

    // Create a customer whose user_id is the reviewer's user_id
    let customer_id = unique("CX_LINKED");
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
         VALUES ('{customer_id}', '{ORG_1}'::uuid, '{customer_id}', 'ReviewerSelf', 'LIVE', now(), '{}')",
        reviewer.user_id
    ))
    .await;

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k1");
    let err = intake
        .open(&reviewer, &customer_id, doc)
        .await
        .expect_err("reviewer linked to customer must be refused");

    assert!(matches!(err, BranchIntakeError::OutOfScope));
    assert_eq!(err.code(), "outOfScope");
    assert_eq!(db.cases_count().await, 0);
    assert_eq!(db.audit_rows().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn branch_reviewer_opens_attaches_and_submits_in_one_transaction() {
    // Design V17, #1398 done criteria 1: branch reviewer opens, attaches and submits in one transaction:
    // case SUBMITTED, branch_id = reviewer's branch, provider STAFF, one document, one audit row.
    let db = Db::start().await;
    let (reviewer, branch_id) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let secret_ident = "PASSPORT-IDENT-123456789";
    db.exec(&format!(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, nationality, status) \
         VALUES (gen_random_uuid(), '{ORG_1}'::uuid, '{}', 'PASSPORT', '{secret_ident}'::bytea, 'k-ident', gen_random_uuid()::text::bytea, 'THA', 'ACTIVE')",
        customer.id
    ))
    .await;
    let intake = db.intake();

    let bytes = png();
    let expected_hash = sha256(&bytes);
    let doc = BranchDocument::new(IdentityType::Passport, bytes.clone(), "k-open-1");

    let audit_before = db.audit_rows().await;
    let (case, document) = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect("branch intake succeeds");

    // Case assertions
    assert_eq!(case.status, CaseStatus::Submitted);
    assert_eq!(case.branch_id, Some(branch_id.clone()));
    assert_eq!(case.provider, Provider::Staff);
    assert_eq!(case.customer_id, customer.id);
    assert_eq!(case.policy_version, 1);
    assert_eq!(case.version, 1);
    assert!(case.reviewed_by.is_none());
    assert!(case.decided_at.is_none());

    // Document assertions
    assert_eq!(document.case_id, case.id);
    assert_eq!(document.document_type, IdentityType::Passport);
    assert_eq!(document.uploaded_by, reviewer.user_id);
    assert_eq!(document.access_class, AccessClass::IdentityDocument);
    assert_eq!(document.mime, "image/png");
    assert_eq!(document.size_bytes as usize, bytes.len());
    assert_eq!(document.checksum, expected_hash);
    assert!(document.encrypted_object_ref.is_some());

    // Database assertions: exactly 1 case, 1 document, 1 audit row
    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 1);
    assert_eq!(db.audit_rows().await, audit_before + 1);

    // Audit row properties (V6: no secret bytes, no identity values)
    let audit_after = db.audit_rows().await;
    assert_eq!(audit_after, audit_before + 1);
    let (audit_json,): (String,) = sqlx::query_as(AssertSqlSafe(
        "SELECT row_to_json(a)::text FROM audit_events a".to_string(),
    ))
    .fetch_one(&db.pool)
    .await
    .expect("fetch audit row");
    let marker_str = std::str::from_utf8(MARKER).unwrap();
    let b64_std = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let b64_url = base64::engine::general_purpose::URL_SAFE.encode(&bytes);
    let hex_lower: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hex_upper: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    assert!(
        !audit_json.contains(marker_str),
        "audit row must not contain secret file marker bytes (V6)"
    );
    assert!(
        !audit_json.contains(&b64_std),
        "audit row must not contain base64 STANDARD of uploaded file bytes (V6)"
    );
    assert!(
        !audit_json.contains(&b64_url),
        "audit row must not contain base64 URL_SAFE of uploaded file bytes (V6)"
    );
    assert!(
        !audit_json.contains(&hex_lower),
        "audit row must not contain lowercase hex of uploaded file bytes (V6)"
    );
    assert!(
        !audit_json.contains(&hex_upper),
        "audit row must not contain uppercase hex of uploaded file bytes (V6)"
    );
    assert!(
        !audit_json.contains(secret_ident),
        "audit row must not contain customer identity values (V6)"
    );

    // Store assertion: exactly 1 stored object matching uploaded bytes
    let objects = db.store.objects();
    assert_eq!(objects.len(), 1);
    let stored_bytes = objects
        .get(document.encrypted_object_ref.as_ref().unwrap())
        .expect("object stored");
    assert_eq!(stored_bytes, &bytes);

    db.finish().await;
}

#[tokio::test]
async fn national_id_is_accepted_under_policy_version_1() {
    // Design V11, OQ-1: {PASSPORT, NATIONAL_ID} are accepted under policy version 1.
    let db = Db::start().await;
    let (reviewer, branch_id) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let intake = db.intake();

    let bytes = pdf();
    let doc = BranchDocument::new(IdentityType::NationalId, bytes.clone(), "k-nid-1");
    let (case, document) = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect("national id succeeds");

    assert_eq!(case.status, CaseStatus::Submitted);
    assert_eq!(case.branch_id, Some(branch_id));
    assert_eq!(document.document_type, IdentityType::NationalId);
    assert_eq!(document.mime, "application/pdf");
    db.finish().await;
}

#[tokio::test]
async fn unaccepted_document_type_phone_is_refused_document_type_not_accepted() {
    // Design §4.3, #1398 §9 edge cases: documentType PHONE → documentTypeNotAccepted.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let intake = db.intake();

    let doc = BranchDocument::new(IdentityType::Phone, png(), "k-phone");
    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("PHONE document must be refused");

    assert!(matches!(err, BranchIntakeError::DocumentTypeNotAccepted));
    assert_eq!(err.code(), "documentTypeNotAccepted");
    assert_eq!(db.cases_count().await, 0);
    assert_eq!(db.documents_count().await, 0);
    assert_eq!(db.audit_rows().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn file_larger_than_15mb_is_refused_with_validation_failed() {
    // Design §4.3, file_service.rs:12 MAX_UPLOAD_BYTES: size > 15 MB → TOO_LARGE / validationFailed.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let intake = db.intake();

    let mut large = vec![0u8; MAX_UPLOAD_BYTES + 1];
    large[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    let doc = BranchDocument::new(IdentityType::Passport, large, "k-large");

    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("upload exceeding 15 MB must be refused");

    assert!(matches!(err, BranchIntakeError::File(_)));
    assert_eq!(err.code(), "validationFailed");
    assert!(
        err.to_string().contains(TOO_LARGE) || err.to_string().contains("15 MB"),
        "error must cite file size limit: {err}"
    );
    assert_eq!(db.cases_count().await, 0);
    db.finish().await;
}

/// BI1 (branch-intake empty file): a 0-byte upload is told the file is empty, as the customer route says since
/// #4925, not the 15 MB sentence.
#[tokio::test]
async fn bi1_an_empty_file_is_refused_as_empty_not_as_too_large() {
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let intake = db.intake();

    let doc = BranchDocument::new(IdentityType::Passport, Vec::new(), "k-empty");

    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("BI1: an empty upload must be refused");

    assert!(matches!(err, BranchIntakeError::File(_)));
    assert_eq!(err.code(), "validationFailed");
    assert!(
        err.to_string().contains("the file is empty"),
        "BI1: an empty file is told it is empty: {err}"
    );
    assert!(
        !err.to_string().contains("15 MB"),
        "BI1: an empty file is not told it is over 15 MB: {err}"
    );
    assert_eq!(db.cases_count().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn unsupported_file_content_is_refused_with_validation_failed() {
    // Design §4.3, file_service.rs content check: non-image/pdf content judged by content.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let intake = db.intake();

    let doc = BranchDocument::new(IdentityType::Passport, invalid_bytes(), "k-invalid");
    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("invalid file bytes must be refused");

    assert!(matches!(err, BranchIntakeError::File(_)));
    assert_eq!(err.code(), "validationFailed");
    assert_eq!(db.cases_count().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn existing_open_case_in_draft_attaches_and_submits() {
    // Design V3, V14, §4.3: intake against an open DRAFT case attaches document and submits it.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;

    let existing_case_id = db.force_open_case(&customer.id, ORG_1, "DRAFT").await;
    assert_eq!(db.cases_count().await, 1);

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k-open-draft");
    let (case, document) = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect("intake against open DRAFT case attaches and submits");

    assert_eq!(case.id, existing_case_id);
    assert_eq!(case.status, CaseStatus::Submitted);
    let expected_branch = match &reviewer.scope {
        ReviewerScope::Branch { branch_id, .. } => branch_id.clone(),
        _ => unreachable!(),
    };
    assert_eq!(case.branch_id, Some(expected_branch.clone()));
    assert_eq!(document.case_id, existing_case_id);
    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 1);

    let (status, branch_id): (String, Option<String>) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT status, branch_id FROM verification_cases WHERE id = '{existing_case_id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("fetch case");
    assert_eq!(status, "SUBMITTED");
    assert_eq!(
        branch_id.as_deref(),
        Some(expected_branch.as_str()),
        "verification_cases.branch_id must be the intake branch for existing case"
    );

    db.finish().await;
}

#[tokio::test]
async fn existing_open_case_in_needs_update_attaches_and_submits() {
    // Design V3, V14, §4.3: intake against an open NEEDS_UPDATE case attaches document and submits it.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;

    let existing_case_id = db
        .force_open_case(&customer.id, ORG_1, "NEEDS_UPDATE")
        .await;
    assert_eq!(db.cases_count().await, 1);

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k-open-needs-update");
    let (case, document) = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect("intake against open NEEDS_UPDATE case attaches and submits");

    assert_eq!(case.id, existing_case_id);
    assert_eq!(case.status, CaseStatus::Submitted);
    let expected_branch = match &reviewer.scope {
        ReviewerScope::Branch { branch_id, .. } => branch_id.clone(),
        _ => unreachable!(),
    };
    assert_eq!(case.branch_id, Some(expected_branch.clone()));
    assert_eq!(document.case_id, existing_case_id);
    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 1);

    let (status, branch_id): (String, Option<String>) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT status, branch_id FROM verification_cases WHERE id = '{existing_case_id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("fetch case");
    assert_eq!(status, "SUBMITTED");
    assert_eq!(
        branch_id.as_deref(),
        Some(expected_branch.as_str()),
        "verification_cases.branch_id must be the intake branch for existing case"
    );

    db.finish().await;
}

#[tokio::test]
async fn existing_open_case_in_submitted_refuses_case_locked() {
    // Design V3, V14, §4.3: intake against an open SUBMITTED case refuses with caseLocked.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;

    let existing_case_id = db.force_open_case(&customer.id, ORG_1, "SUBMITTED").await;
    assert_eq!(db.cases_count().await, 1);

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k-open-submitted");
    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("must refuse when customer already has an open SUBMITTED case");

    assert!(matches!(err, BranchIntakeError::CaseLocked));
    assert_eq!(err.code(), "caseLocked");
    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 0);

    let (status,): (String,) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT status FROM verification_cases WHERE id = '{existing_case_id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("fetch case");
    assert_eq!(status, "SUBMITTED");

    db.finish().await;
}

#[tokio::test]
async fn existing_open_case_in_under_review_refuses_case_locked() {
    // Design V3, V14, §4.3: intake against an open UNDER_REVIEW case refuses with caseLocked.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;

    let existing_case_id = db
        .force_open_case(&customer.id, ORG_1, "UNDER_REVIEW")
        .await;
    assert_eq!(db.cases_count().await, 1);

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k-open-under-review");
    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("must refuse when customer already has an open UNDER_REVIEW case");

    assert!(matches!(err, BranchIntakeError::CaseLocked));
    assert_eq!(err.code(), "caseLocked");
    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 0);

    let (status,): (String,) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT status FROM verification_cases WHERE id = '{existing_case_id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("fetch case");
    assert_eq!(status, "UNDER_REVIEW");

    db.finish().await;
}

#[tokio::test]
async fn organisation_without_policy_is_refused_no_verification_policy() {
    // Design V11, §4.3: organisation created after M1 has no policy row → noVerificationPolicy.
    let db = Db::start().await;
    let new_org = db.organisation().await.to_string();
    let (reviewer, _) = db.branch_reviewer(&new_org).await;
    let customer = db.customer(&new_org).await;

    let intake = db.intake();
    let doc = BranchDocument::new(IdentityType::Passport, png(), "k-nopolicy");
    let err = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect_err("organisation without policy must be refused");

    assert!(matches!(err, BranchIntakeError::NoVerificationPolicy));
    assert_eq!(err.code(), "noVerificationPolicy");
    assert_eq!(db.cases_count().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn idempotency_same_key_same_payload_returns_existing_without_duplicate_writes() {
    // Design V14, #1398 done criteria 1: same Idempotency-Key → one case, one document, one object.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let intake = db.intake();

    let bytes = png();
    let doc1 = BranchDocument::new(IdentityType::Passport, bytes.clone(), "idem-key-1");
    let (case1, document1) = intake
        .open(&reviewer, &customer.id, doc1)
        .await
        .expect("first intake");

    let doc2 = BranchDocument::new(IdentityType::Passport, bytes.clone(), "idem-key-1");
    let (case2, document2) = intake
        .open(&reviewer, &customer.id, doc2)
        .await
        .expect("replay intake");

    assert_eq!(case1.id, case2.id, "same case returned");
    assert_eq!(document1.id, document2.id, "same document returned");

    assert_eq!(db.cases_count().await, 1, "exactly one case in DB");
    assert_eq!(db.documents_count().await, 1, "exactly one document in DB");
    assert_eq!(db.audit_rows().await, 1, "exactly one audit row");
    assert_eq!(db.store.objects().len(), 1, "exactly one vault object");
    assert_eq!(
        db.store.puts.load(Ordering::Relaxed),
        1,
        "exactly one store put call across replays"
    );

    db.finish().await;
}

#[tokio::test]
async fn idempotency_same_key_different_payload_is_refused_idempotency_conflict() {
    // Design V14, Blueprint §09: same key with different request hash → idempotencyConflict.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;
    let intake = db.intake();

    let doc1 = BranchDocument::new(IdentityType::Passport, png(), "idem-conflict-key");
    let _ = intake
        .open(&reviewer, &customer.id, doc1)
        .await
        .expect("first intake");

    let doc2 = BranchDocument::new(IdentityType::Passport, jpeg(), "idem-conflict-key");
    let err = intake
        .open(&reviewer, &customer.id, doc2)
        .await
        .expect_err("different payload on same key must fail conflict");

    assert!(matches!(err, BranchIntakeError::IdempotencyConflict));
    assert_eq!(err.code(), "idempotencyConflict");
    assert_eq!(db.cases_count().await, 1);
    assert_eq!(db.documents_count().await, 1);
    assert_eq!(db.audit_rows().await, 1);

    db.finish().await;
}

#[tokio::test]
async fn failure_after_store_write_rolls_back_and_retry_writes_one_object() {
    // Design V14, verification.md:99, #1398 §9 edge cases:
    // A failure after the object write rolls back the row; a retry with the same key writes one object.
    // The case id survives the rollback.
    let db = Db::start().await;
    let (reviewer, _) = db.branch_reviewer(ORG_1).await;
    let customer = db.customer(ORG_1).await;

    let should_fail = Arc::new(AtomicBool::new(true));
    let failing_audit = Arc::new(FailingAuditLog {
        inner: Arc::new(PgAuditLog::new(db.pool.clone())),
        should_fail: should_fail.clone(),
    });
    let intake = db.intake_with_audit_log(failing_audit);

    let bytes = png();
    let doc = BranchDocument::new(IdentityType::Passport, bytes.clone(), "k-rollback-retry");

    // First attempt fails during audit log append after document store write
    let err = intake
        .open(&reviewer, &customer.id, doc.clone())
        .await
        .expect_err("intake must fail when audit fails");
    let _ = err;

    // Database rows are rolled back completely
    assert_eq!(db.cases_count().await, 0, "no case row after rollback");
    assert_eq!(
        db.documents_count().await,
        0,
        "no document row after rollback"
    );
    assert_eq!(db.audit_rows().await, 0, "no audit row after rollback");

    // Store already recorded the object
    assert_eq!(
        db.store.puts.load(Ordering::Relaxed),
        1,
        "one store put before retry"
    );
    assert_eq!(
        db.store.objects().len(),
        1,
        "one stored object before retry"
    );

    // Retry with the SAME key and payload
    should_fail.store(false, Ordering::Relaxed);
    let (case, document) = intake
        .open(&reviewer, &customer.id, doc)
        .await
        .expect("retry must succeed");

    assert_eq!(case.status, CaseStatus::Submitted);
    assert_eq!(document.case_id, case.id);

    // Database now holds exactly 1 row each
    assert_eq!(
        db.cases_count().await,
        1,
        "exactly 1 case in DB after retry"
    );
    assert_eq!(
        db.documents_count().await,
        1,
        "exactly 1 document in DB after retry"
    );
    assert_eq!(
        db.audit_rows().await,
        1,
        "exactly 1 audit row in DB after retry"
    );

    // Exactly 1 vault object across retry
    assert_eq!(
        db.store.objects().len(),
        1,
        "exactly 1 vault object in store"
    );

    db.finish().await;
}
