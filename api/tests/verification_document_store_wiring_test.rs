//! Up-front specification tests for identity document store wiring (#1602).
//!
//! Tests the 5 rows of §9 Behaviour Table:
//! 1. Settings loaded from base.yaml: verification.store_dir == "verification-documents"
//! 2. AppState::new with installed PII keys: documents writes under store_dir; the file on disk is not the plaintext bytes
//! 3. document_ref(case, key): accepted by FsDocumentStore (put then get round-trips)
//! 4. VF-3 CaseService::add_document through the real FsDocumentStore: succeeds, encrypted_object_ref equals document_ref(case, key), get returns the bytes
//! 5. key containing /, .. or starting with .: document_ref refuses it, never builds a ref the store would refuse later

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, migrate::Migrator};
use uuid::Uuid;

use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError, document_ref};
use xc_kernel::pii::PiiCipher;
use xc_kernel::pii::identity_type::IdentityType;
use xc_verification::application::case_service::CaseService;
use xc_verification::domain::case::case_actor::CaseActor;
use xc_verification::domain::case::customer_actor::CustomerActor;
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;
use xchange::bootstrap::AppState;
use xchange::config::Settings;
use xchange::infrastructure::document_store::FsDocumentStore;

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options: PgConnectOptions = url.parse().expect("valid PostgreSQL URL");
        let database = options.get_database().unwrap_or("");
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL database name '{database}' must end in `_test`"
        );
        let schema = format!("vf_store_{}", Uuid::new_v4().simple());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options.clone())
            .await
            .expect("connect to admin test DB");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema pool");

        let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let migrator = Migrator::new(migrations_path)
            .await
            .expect("load migrations");
        migrator.run(&pool).await.expect("run migrations");

        Self { pool, schema }
    }

    async fn finish(self) {
        let admin_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let options: PgConnectOptions = admin_url.parse().expect("valid URL");
        self.pool.close().await;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options)
            .await
            .expect("connect to drop schema");
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await;
        admin.close().await;
    }
}

// ── Row 1: Settings loaded from base.yaml ────────────────────────────────────

#[test]
fn settings_loads_verification_store_dir_from_base_yaml() {
    let settings = Settings::load_from(&common::config_dir(), common::env(&[]))
        .expect("base.yaml must load without error");
    assert_eq!(
        settings.verification.store_dir,
        PathBuf::from("verification-documents"),
        "verification.store_dir must be 'verification-documents' as specified in base.yaml:31-34"
    );
}

// ── Row 2: AppState::new with installed PII keys ────────────────────────────

#[tokio::test]
async fn app_state_new_constructs_documents_store_writing_under_store_dir_sealed() {
    common::install_test_pii_keys();
    let temp = TempDir::new("xc-store-state");

    let mut settings = common::settings(&[]);
    settings.verification.store_dir = temp.path().to_path_buf();

    let state = AppState::new(Arc::new(settings), common::unreachable_pool());

    let doc_ref = "test_doc_ref_123";
    let plaintext = b"CUSTOMER-PASSPORT-CONFIDENTIAL-IMAGE-BYTES";

    state
        .document_store
        .put(doc_ref, plaintext)
        .await
        .expect("documents.put must succeed");

    let target_file = temp.path().join(doc_ref);
    assert!(
        target_file.exists(),
        "file must be written on disk under settings.verification.store_dir"
    );

    let disk_bytes = std::fs::read(&target_file).expect("read file from disk");
    assert_ne!(
        disk_bytes, plaintext,
        "the file on disk is sealed: must NOT be the plaintext bytes"
    );
    assert!(
        !disk_bytes.windows(plaintext.len()).any(|w| w == plaintext),
        "disk bytes must not contain the plaintext payload anywhere"
    );

    let retrieved = state
        .document_store
        .get(doc_ref)
        .await
        .expect("documents.get must decrypt and return the object");
    assert_eq!(retrieved, plaintext, "retrieved bytes must match plaintext");
}

// ── Row 3: document_ref(case, key) accepted by FsDocumentStore ──────────────

#[tokio::test]
async fn document_ref_accepted_by_fs_document_store_round_trip() {
    common::install_test_pii_keys();
    let temp = TempDir::new("xc-store-ref");

    let keys = xc_customer::get_pii_keys().expect("installed test PII keys");
    let cipher = PiiCipher::new(keys);
    let store = FsDocumentStore::new(temp.path(), cipher);

    let case_id = Uuid::new_v4();
    let idempotency_key = "upload-idem-key-42";
    let doc_ref = document_ref(&case_id.to_string(), idempotency_key)
        .expect("document_ref must accept a valid idempotency key");

    assert_eq!(
        doc_ref,
        format!("{case_id}_{idempotency_key}"),
        "document_ref must shape as {{case}}_{{key}}"
    );

    let payload = b"PNG-DOCUMENT-CONTENT-BYTES";
    let key_id = store
        .put(&doc_ref, payload)
        .await
        .expect("FsDocumentStore must accept document_ref and store it");
    assert!(!key_id.is_empty());

    let fetched = store
        .get(&doc_ref)
        .await
        .expect("FsDocumentStore must retrieve object stored with document_ref");
    assert_eq!(fetched, payload, "round-trip payload must match");
}

// ── Row 4: VF-3 CaseService::add_document through real FsDocumentStore ──────

#[tokio::test]
async fn case_service_add_document_through_real_fs_document_store_stores_document_ref() {
    common::install_test_pii_keys();
    let temp = TempDir::new("xc-case-doc-store");

    let keys = xc_customer::get_pii_keys().expect("installed test PII keys");
    let cipher = PiiCipher::new(keys);
    let store = Arc::new(FsDocumentStore::new(temp.path(), cipher));

    let db = TestDb::start().await;

    let org_id = Uuid::new_v4();
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO organizations (id, code, timezone, created_at) VALUES ('{org_id}', 'org_{}', 'Asia/Bangkok', now())",
        Uuid::new_v4().simple()
    )))
    .execute(&db.pool)
    .await
    .expect("insert org");

    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO verification_policies (organization_id, version, accepted_document_types, documents_required, branch_review_allowed, published_by, published_at, created_at) \
         VALUES ('{org_id}', 1, ARRAY['PASSPORT', 'NATIONAL_ID'], 1, true, NULL, now(), now())"
    )))
    .execute(&db.pool)
    .await
    .expect("insert policy");

    let user_id = Uuid::new_v4();
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) VALUES ('{user_id}', '{org_id}', 'User', 'ACTIVE', 'test')"
    )))
    .execute(&db.pool)
    .await
    .expect("insert user");

    let customer_id = format!("cx_{}", Uuid::new_v4().simple());
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
         VALUES ('{customer_id}', '{org_id}', '{customer_id}', 'Customer', 'LIVE', now(), '{user_id}')"
    )))
    .execute(&db.pool)
    .await
    .expect("insert customer");

    let actor = CaseActor::Customer(CustomerActor {
        customer_id: customer_id.clone(),
    });

    let case_repo = Arc::new(PgCaseRepository::new(db.pool.clone()));
    let audit_log = Arc::new(PgAuditLog::new(db.pool.clone()));
    let svc = CaseService::new(db.pool.clone(), case_repo, store.clone(), audit_log);

    let case = svc
        .open(&actor, &customer_id)
        .await
        .expect("case open must succeed");

    // Real PNG magic bytes (8 bytes) + arbitrary payload
    let png_bytes = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x11, 0x22, 0x33, 0x44,
    ];
    let idempotency_key = "doc-idem-key-77";

    let doc = svc
        .add_document(
            &actor,
            case.id,
            IdentityType::Passport,
            &png_bytes,
            idempotency_key,
        )
        .await
        .expect("CaseService::add_document must succeed through real FsDocumentStore");

    let expected_ref =
        document_ref(&case.id.to_string(), idempotency_key).expect("document_ref must succeed");
    assert_eq!(
        doc.encrypted_object_ref.as_deref(),
        Some(expected_ref.as_str()),
        "encrypted_object_ref must equal document_ref(case, key)"
    );

    let retrieved = store
        .get(&expected_ref)
        .await
        .expect("FsDocumentStore must retrieve object using the ref");
    assert_eq!(
        retrieved, png_bytes,
        "store.get must return the original document bytes"
    );

    db.finish().await;
}

// ── Row 5: document_ref refuses invalid keys ────────────────────────────────

#[test]
fn document_ref_refuses_invalid_keys_with_slash_dot_dot_or_leading_dot() {
    let case_id = Uuid::new_v4().to_string();

    // Key containing '/'
    let res_slash = document_ref(&case_id, "folder/file");
    assert!(
        matches!(res_slash, Err(DocumentStoreError::InvalidRef(_))),
        "key containing '/' must be refused with InvalidRef, got {res_slash:?}"
    );

    // Key containing '..'
    let res_dotdot = document_ref(&case_id, "../parent");
    assert!(
        matches!(res_dotdot, Err(DocumentStoreError::InvalidRef(_))),
        "key containing '..' must be refused with InvalidRef, got {res_dotdot:?}"
    );

    // Key starting with '.'
    let res_dot = document_ref(&case_id, ".hidden");
    assert!(
        matches!(res_dot, Err(DocumentStoreError::InvalidRef(_))),
        "key starting with '.' must be refused with InvalidRef, got {res_dot:?}"
    );

    // Key with empty
    let res_empty = document_ref(&case_id, "");
    assert!(
        matches!(res_empty, Err(DocumentStoreError::InvalidRef(_))),
        "empty key must be refused with InvalidRef, got {res_empty:?}"
    );

    // Key containing '..' anywhere ("a..b")
    let res_a_dotdot_b = document_ref(&case_id, "a..b");
    assert!(
        matches!(res_a_dotdot_b, Err(DocumentStoreError::InvalidRef(_))),
        "key containing '..' anywhere must be refused with InvalidRef, got {res_a_dotdot_b:?}"
    );

    // Key containing '\' ("a\\b")
    let res_backslash = document_ref(&case_id, "a\\b");
    assert!(
        matches!(res_backslash, Err(DocumentStoreError::InvalidRef(_))),
        "key containing '\\' must be refused with InvalidRef, got {res_backslash:?}"
    );

    // Key containing NUL ("a\0b")
    let res_nul = document_ref(&case_id, "a\0b");
    assert!(
        matches!(res_nul, Err(DocumentStoreError::InvalidRef(_))),
        "key containing NUL must be refused with InvalidRef, got {res_nul:?}"
    );
}
