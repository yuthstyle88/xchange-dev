//! Up-front failing tests — Idempotency-Key has no length bound.
//!
//! Today the upload routes check only the key's characters (#2618, `key_shape_ok`) and every v3
//! command checks only that it is non-empty. The upload ref is `<36-char case uuid>_<key>` and
//! `FsDocumentStore` uses it as a file name, so a key over ~218 bytes overflows the 255-byte file
//! name limit (`DocumentStoreError::Io` -> 500). A v3 command puts the key into the
//! `idempotency_records` primary key, so an 8,000-byte key overflows the btree row limit (2704)
//! and the claim fails -> 500 "idempotency claim failed".
//!
//! Behaviour table (limit = 128 bytes):
//! L1: app verification upload, 300-char key -> 400 `validationFailed` field `idempotencyKey`; nothing stored (today 500)
//! L2: branch verification intake, 300-char key -> 400 `badRequest`; nothing stored (today 500)
//! L3: v3 command (PUT /api/retention-policies/{dataType}), 8,000-char key -> 400 `validationFailed`
//!     field `idempotencyKey`; no idempotency record, policy unchanged (today 500)
//! L4 (guard): app verification upload, 128-char key of valid characters -> 200, one document stored

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{PiiCipher, PiiKeys};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::document_store::FsDocumentStore;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS_APP: &str = "x-test-as";
const AS_BRANCH: &str = "x-test-signed-in-as";
const HOST: &str = "xc.test";
const ORIGIN: &str = "http://xc.test";

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "B01";
const USER_CUST_1: &str = "55555555-5555-5555-5555-555555555555";
const USER_STAFF_1: &str = "66666666-6666-6666-6666-666666666666";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn valid_png_bytes() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(b"CONFIDENTIAL_CUSTOMER_PASSPORT_PNG_PAYLOAD");
    b
}

fn make_multipart(
    boundary: &str,
    fields: &[(&str, &str)],
    file: Option<(&str, &str, &str, &[u8])>,
) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, val) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{val}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some((field_name, filename, mime, data)) = file {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec_app = req
            .headers()
            .get(AS_APP)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let spec_branch = req
            .headers()
            .get(AS_BRANCH)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();

        Box::pin(async move {
            let user = if !spec_app.is_empty() {
                let parts: Vec<&str> = spec_app.split('|').collect();
                if parts.len() >= 5 {
                    let user_id = parts[0];
                    let role = Role::parse(parts[1]);
                    let branch = if parts[2].is_empty() {
                        None
                    } else {
                        Some(BranchId::new(parts[2]))
                    };
                    let customer_id = if parts[3].is_empty() {
                        None
                    } else {
                        Some(parts[3])
                    };
                    let org_id = parts[4];
                    role.map(|r| {
                        let from = Utc::now() - ChronoDuration::days(30);
                        let scoped = ScopedRole::new(r, branch.clone(), from, None);
                        let mut actor = User::new(user_id, org_id, vec![scoped], customer_id);
                        actor.branch = branch;
                        actor
                    })
                } else {
                    None
                }
            } else if !spec_branch.is_empty() {
                let mut parts = spec_branch.splitn(4, '|');
                let id = parts.next().unwrap_or_default();
                let role = parts.next().unwrap_or_default();
                let branch = parts.next().unwrap_or_default();
                let org = parts.next().unwrap_or(ORG_1);

                Role::parse(role).map(|r| {
                    let branch_id = (branch != "*").then(|| BranchId::new(branch));
                    let from = Utc::now() - ChronoDuration::days(30);
                    let mut actor = User::new(
                        id,
                        org,
                        vec![ScopedRole::new(r, branch_id.clone(), from, None)],
                        None,
                    );
                    actor.branch = branch_id;
                    actor
                })
            } else {
                None
            };

            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct CountingStore {
    inner: Arc<dyn DocumentStore>,
    puts: AtomicU64,
}

type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

impl DocumentStore for CountingStore {
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> StoreFuture<'a, Result<String, DocumentStoreError>> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.inner.put(doc_ref, bytes)
    }

    fn get<'a>(&'a self, doc_ref: &'a str) -> StoreFuture<'a, Result<Vec<u8>, DocumentStoreError>> {
        self.inner.get(doc_ref)
    }

    fn delete<'a>(&'a self, doc_ref: &'a str) -> StoreFuture<'a, Result<(), DocumentStoreError>> {
        self.inner.delete(doc_ref)
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

struct TestEnv {
    pool: PgPool,
    schema: String,
    store_dir: std::path::PathBuf,
    store: Arc<CountingStore>,
}

impl TestEnv {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let schema = unique("keylen");
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
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema pool");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        let store_dir = std::env::temp_dir().join(format!("xc-docstore-keylen-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&store_dir).expect("create temp docstore dir");
        let keys = Arc::new(
            PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("valid test keys"),
        );
        let fs_store = Arc::new(FsDocumentStore::new(
            store_dir.clone(),
            PiiCipher::new(keys),
        ));
        let store = Arc::new(CountingStore {
            inner: fs_store,
            puts: AtomicU64::new(0),
        });

        let env = Self {
            pool,
            schema,
            store_dir,
            store,
        };
        env.seed().await;
        env
    }

    async fn seed(&self) {
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, 'ORG1', 'First Org', 'Asia/Bangkok', now()) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed org1");

        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(B01)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed branch");

        sqlx::query(
            "INSERT INTO verification_policies \
             (organization_id, version, accepted_document_types, documents_required, branch_review_allowed, published_by, published_at, created_at) \
             VALUES ($1, 1, ARRAY['PASSPORT', 'NATIONAL_ID'], 1, true, NULL, now(), now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed org1 policy");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by, created_at) VALUES \
             ($1, $2, 'Customer Linked User', 'ACTIVE', 'seed', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed customer user");

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) VALUES \
             ('CX_1', $1, 'CX_1', 'Somchai Prasert', 'LIVE', now(), $2) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed customer CX_1");

        let secret_ident = "PASSPORT-IDENT-123456789";
        sqlx::query(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, nationality, status) \
             VALUES (gen_random_uuid(), $1, 'CX_1', 'PASSPORT', $2, 'k-ident', gen_random_uuid()::text::bytea, 'THA', 'ACTIVE')",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(secret_ident.as_bytes())
        .execute(&self.pool)
        .await
        .expect("seed customer identity");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by, created_at) VALUES \
             ($1, $2, 'Staff Reviewer User', 'ACTIVE', 'seed', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::from_str(USER_STAFF_1).unwrap())
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed staff user");
    }

    async fn insert_draft_case(&self, case_id: Uuid, customer_id: &str) {
        sqlx::query(
            "INSERT INTO verification_cases \
             (id, organization_id, customer_id, provider, status, policy_version, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'STAFF', 'DRAFT', 1, 1, now(), now())",
        )
        .bind(case_id)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(customer_id)
        .execute(&self.pool)
        .await
        .expect("insert draft case");
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn call_app_upload(&self, case_id: Uuid, idempotency_key: Option<&str>) -> Answer {
        let mut state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        state.document_store = self.store.clone();
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;

        let boundary = "---------------------------appuploadboundary";
        let png = valid_png_bytes();
        let payload = make_multipart(
            boundary,
            &[("documentType", "PASSPORT")],
            Some(("file", "passport.png", "image/png", &png)),
        );

        let uri = format!("/api/app/verification/cases/{case_id}/documents");
        let spec = format!("{USER_CUST_1}|CUSTOMER||CX_1|{ORG_1}|session-1");
        let mut req = atest::TestRequest::default()
            .method(Method::POST)
            .uri(&uri)
            .insert_header((header::HOST, HOST))
            .insert_header(("origin", ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((AS_APP, spec))
            .insert_header((header::COOKIE, "__Host-xc_session=session-1"))
            .insert_header((
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(payload);

        if let Some(key) = idempotency_key {
            req = req.insert_header(("idempotency-key", key.to_owned()));
        }

        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        Answer { status, text, body }
    }

    async fn call_intake(&self, idempotency_key: Option<&str>) -> Answer {
        let mut state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        state.document_store = self.store.clone();
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;

        let boundary = "---------------------------intakeboundary";
        let png = valid_png_bytes();
        let payload = make_multipart(
            boundary,
            &[("customerId", "CX_1"), ("documentType", "PASSPORT")],
            Some(("file", "passport.png", "image/png", &png)),
        );

        let spec = format!("{USER_STAFF_1}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");
        let mut req = atest::TestRequest::default()
            .method(Method::POST)
            .uri("/api/operations/verification/branch-cases")
            .insert_header((header::HOST, HOST))
            .insert_header(("origin", "https://xc.test"))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("x-forwarded-proto", "https"))
            .insert_header((AS_BRANCH, spec))
            .insert_header((
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(payload);

        if let Some(key) = idempotency_key {
            req = req.insert_header(("idempotency-key", key.to_owned()));
        }

        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        Answer { status, text, body }
    }

    async fn finish(self) {
        let TestEnv {
            pool,
            schema,
            store_dir,
            ..
        } = self;
        let _ = std::fs::remove_dir_all(&store_dir);
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

/// A key of `len` ASCII alphanumerics. Pseudo-random so PostgreSQL's index compression cannot
/// shrink it below the btree row limit (a run of one letter compresses to almost nothing).
fn long_key(len: usize, seed: u64) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ALPHABET[((state >> 33) % ALPHABET.len() as u64) as usize] as char
        })
        .collect()
}

// ── Behaviour table tests ───────────────────────────────────────────────────

#[actix_web::test]
async fn idempotency_key_length_l1_app_upload_refuses_a_300_char_key() {
    let env = TestEnv::start().await;
    let case_id = Uuid::new_v4();
    env.insert_draft_case(case_id, "CX_1").await;

    let key = long_key(300, 1);
    let answer = env.call_app_upload(case_id, Some(&key)).await;
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "L1: a 300-char Idempotency-Key on the app upload must be refused with 400, got: {} {}",
        answer.status,
        answer.text
    );
    assert_eq!(answer.body["error"]["code"], "validationFailed");
    assert_eq!(answer.body["error"]["field"], "idempotencyKey");
    assert_eq!(
        env.count("SELECT count(*) FROM verification_documents")
            .await,
        0,
        "no verification_documents row may be stored"
    );
    assert_eq!(
        env.store.puts.load(Ordering::SeqCst),
        0,
        "no file may be stored in the document store"
    );

    env.finish().await;
}

#[actix_web::test]
async fn idempotency_key_length_l2_branch_intake_refuses_a_300_char_key() {
    let env = TestEnv::start().await;

    let key = long_key(300, 2);
    let answer = env.call_intake(Some(&key)).await;
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "L2: a 300-char Idempotency-Key on the branch intake must be refused with 400, got: {} {}",
        answer.status,
        answer.text
    );
    assert_eq!(answer.body["error"]["code"], "badRequest");
    assert_eq!(
        env.count("SELECT count(*) FROM verification_documents")
            .await,
        0,
        "no verification_documents row may be stored"
    );
    assert_eq!(
        env.count("SELECT count(*) FROM verification_cases").await,
        0,
        "no verification_cases row may be stored"
    );
    assert_eq!(
        env.store.puts.load(Ordering::SeqCst),
        0,
        "no file may be stored in the document store"
    );

    env.finish().await;
}

#[actix_web::test]
async fn idempotency_key_length_l3_v3_command_refuses_an_8000_char_key() {
    struct Owner(String);
    impl SessionReader for Owner {
        fn callers<'a>(
            &'a self,
            _req: &'a HttpRequest,
        ) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
            let id = self.0.clone();
            Box::pin(async move { test_callers::callers(Some(test_callers::owner(&id)), None) })
        }
    }

    let env = TestEnv::start().await;
    for b in ["MAIN", "XC-B01"] {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING")
            .bind(b)
            .execute(&env.pool)
            .await
            .expect("insert a branch");
    }
    let owner_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT organization_id, 'Owner User', 'ACTIVE', 'test' FROM branches WHERE id = 'MAIN' \
         RETURNING id::text",
    )
    .fetch_one(&env.pool)
    .await
    .expect("insert the owner user");
    let policy_sql = "SELECT count(*) FROM retention_policies \
                      WHERE data_type = 'IDENTITY_VALUE' AND period_days = 61";
    let before = env.count(policy_sql).await;

    let state = AppState::new(Arc::new(common::settings(&[])), env.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Owner(owner_id));
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let key = long_key(8_000, 3);
    let req = atest::TestRequest::default()
        .method(Method::PUT)
        .uri("/api/retention-policies/IDENTITY_VALUE")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(json!({"periodDays": 61}).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "L3: an 8,000-char Idempotency-Key on a v3 command must be refused with 400, got: {status} {text}"
    );
    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "idempotencyKey");
    assert_eq!(
        env.count("SELECT count(*) FROM idempotency_records").await,
        0,
        "no idempotency record may be stored"
    );
    assert_eq!(
        env.count(policy_sql).await,
        before,
        "the retention policy must not change"
    );

    env.finish().await;
}

#[actix_web::test]
async fn idempotency_key_length_l4_app_upload_accepts_a_128_char_key() {
    let env = TestEnv::start().await;
    let case_id = Uuid::new_v4();
    env.insert_draft_case(case_id, "CX_1").await;

    let key = long_key(128, 4);
    assert_eq!(key.len(), 128);
    let answer = env.call_app_upload(case_id, Some(&key)).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "L4: a 128-char Idempotency-Key of valid characters must be accepted, got: {} {}",
        answer.status,
        answer.text
    );
    assert_eq!(
        env.count("SELECT count(*) FROM verification_documents")
            .await,
        1,
        "exactly one verification_documents row must be stored"
    );
    assert_eq!(
        env.store.puts.load(Ordering::SeqCst),
        1,
        "exactly one file must be stored in the document store"
    );

    env.finish().await;
}
