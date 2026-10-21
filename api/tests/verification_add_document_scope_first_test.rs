//! Ticket #2706: Customer verification: `add_document` checks idempotency before ownership,
//! answering another customer's document on a key collision.
//!
//! Behaviour table (§8):
//! - D1: CX_2 -> CX_1's case, CX_1's key, the same bytes: 403 outOfScope, no row (main: 200)
//! - D2: CX_2 -> CX_1's case, CX_1's key, different bytes: 403 outOfScope (main: 409)
//! - D3: CX_1 replays its own key with the same bytes: 200, the same document (kept)
//! - D4: CX_1 replays its own key with different bytes: 409 idempotencyConflict (kept)
//! - D5: unknown case id: 404 notFound (kept)

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::header::HeaderMap;
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::document_store::DocumentStore;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::document_store::FsDocumentStore;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const BRANCH_MAIN: &str = "MAIN";

const USER_REVIEWER: &str = "11111111-1111-1111-1111-111111111111";
const USER_CUST_1: &str = "55555555-5555-5555-5555-555555555555";
const USER_CUST_2: &str = "66666666-6666-6666-6666-666666666666";

const SECRET_DOC_BYTES: &[u8] =
    b"\x89PNG\r\n\x1a\nCONFIDENTIAL_CUSTOMER_PASSPORT_RAW_BYTES_FOR_C10";
const DIFFERENT_DOC_BYTES: &[u8] =
    b"\x89PNG\r\n\x1a\nDIFFERENT_CUSTOMER_PASSPORT_RAW_BYTES_FOR_C10";
const SECRET_IDENTITY_FULL: &str = "BB9876543";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

#[derive(Clone, Debug)]
struct Who {
    user_id: String,
    role: Role,
    branch: Option<String>,
    customer_id: Option<String>,
    org_id: String,
    session: String,
}

impl Who {
    fn customer(user_id: &str, customer_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Customer,
            branch: None,
            customer_id: Some(customer_id.to_owned()),
            org_id: ORG_1.to_owned(),
            session: format!("session-{user_id}"),
        }
    }

    fn spec(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.user_id,
            self.role.as_str(),
            self.branch.as_deref().unwrap_or(""),
            self.customer_id.as_deref().unwrap_or(""),
            self.org_id,
        )
    }
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();

        Box::pin(async move {
            let user = if spec.is_empty() {
                None
            } else {
                let parts: Vec<&str> = spec.split('|').collect();
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
            };

            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

#[allow(dead_code)]
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
    body: Value,
    bytes: Vec<u8>,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
            .to_owned()
    }
}

struct Room {
    pool: PgPool,
    schema: String,
    store_dir: std::path::PathBuf,
    store: Arc<FsDocumentStore>,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let port = options.get_port();
        assert_ne!(
            port, 6381,
            "port 6381 is reserved for queue; use a throwaway PG port"
        );

        let schema = unique("c10_scope_first");
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

        let store_dir = std::env::temp_dir().join(format!("xc-docstore-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&store_dir).expect("create temp docstore dir");
        let keys = Arc::new(
            PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("valid test keys"),
        );
        let store = Arc::new(FsDocumentStore::new(
            store_dir.clone(),
            PiiCipher::new(keys),
        ));

        let room = Self {
            pool,
            schema,
            store_dir,
            store,
        };
        room.seed_defaults().await;
        room
    }

    async fn seed_defaults(&self) {
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, 'ORG1', 'First Org', 'Asia/Bangkok', now()) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed org1");

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
            "INSERT INTO branches (id, organization_id, name) VALUES \
             ($1, $2, 'Main Branch') \
             ON CONFLICT (id) DO UPDATE SET organization_id = EXCLUDED.organization_id",
        )
        .bind(BRANCH_MAIN)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed branches");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ($1, $4, 'Org Reviewer', 'ACTIVE', 'seed'), \
             ($2, $4, 'Customer User 1', 'ACTIVE', 'seed'), \
             ($3, $4, 'Customer User 2', 'ACTIVE', 'seed') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::from_str(USER_REVIEWER).unwrap())
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .bind(Uuid::from_str(USER_CUST_2).unwrap())
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed users");

        for (cookie, user) in [
            ("session-user-cust-1", USER_CUST_1),
            ("session-user-cust-2", USER_CUST_2),
        ] {
            let hash: String = Sha256::digest(cookie.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            sqlx::query(
                "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
                 VALUES ($1, $2::uuid, 1, now() + interval '8 hours', $3) ON CONFLICT (hash) DO NOTHING",
            )
            .bind(hash)
            .bind(user)
            .bind(Uuid::from_str(ORG_1).unwrap())
            .execute(&self.pool)
            .await
            .expect("seed a live session");
        }

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, user_id, created_at) VALUES \
             ('CX_1', $1, 'CX_1', 'Somchai Prasert', 'LIVE', $2, now()), \
             ('CX_2', $1, 'CX_2', 'Somsri Sukjai', 'LIVE', $3, now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .bind(Uuid::from_str(USER_CUST_2).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed customers");

        common::install_test_pii_keys();
        let keys = Arc::new(
            PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("the test PII key parses"),
        );
        let cipher = PiiCipher::new(keys.clone());
        let hasher = LookupHasher::new(&keys);
        let aad = "CX_1PASSPORT";
        let payload = serde_json::json!({
            "number": SECRET_IDENTITY_FULL,
            "nationality": "THA",
            "birthDate": "1990-01-01",
            "expiry": "2030-01-01",
            "nameOnDocument": "Somchai Prasert",
        });
        let sealed = cipher.encrypt(aad.as_bytes(), payload.to_string().as_bytes());
        let lookup_hash = hasher.hash(IdentityType::Passport, SECRET_IDENTITY_FULL);

        sqlx::query(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, nationality, verified_at, status, created_at) \
             VALUES (gen_random_uuid(), $1, 'CX_1', 'PASSPORT', $2, $3, $4, 'THA', now(), 'ACTIVE', now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(&sealed.bytes)
        .bind(&sealed.key_id)
        .bind(&lookup_hash[..])
        .execute(&self.pool)
        .await
        .expect("seed customer identity");
    }

    async fn insert_case(&self, case_id: Uuid, org_id: &str, customer_id: &str, status: &str) {
        sqlx::query(
            "INSERT INTO verification_cases \
             (id, organization_id, customer_id, provider, external_case_ref, status, policy_version, branch_id, \
              submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'STAFF', NULL, $4, 1, NULL, NULL, NULL, NULL, NULL, NULL, 1, now(), now())",
        )
        .bind(case_id)
        .bind(Uuid::from_str(org_id).unwrap())
        .bind(customer_id)
        .bind(status)
        .execute(&self.pool)
        .await
        .expect("insert verification case");
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_document(
        &self,
        doc_id: Uuid,
        org_id: &str,
        case_id: Uuid,
        doc_type: &str,
        object_ref: Option<&str>,
        checksum: &[u8],
        size_bytes: i32,
    ) {
        if let Some(oref) = object_ref {
            self.store
                .put(oref, SECRET_DOC_BYTES)
                .await
                .expect("store test document bytes");
        }

        sqlx::query(
            "INSERT INTO verification_documents \
             (id, organization_id, case_id, document_type, encrypted_object_ref, key_id, checksum, mime, size_bytes, \
              uploaded_at, uploaded_by, retention_until, access_class, removed_at, purged_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, 'test-1', $6, 'image/png', $7, now(), $8, now() + interval '365 days', 'IDENTITY_DOCUMENT', NULL, NULL, now())",
        )
        .bind(doc_id)
        .bind(Uuid::from_str(org_id).unwrap())
        .bind(case_id)
        .bind(doc_type)
        .bind(object_ref)
        .bind(checksum)
        .bind(size_bytes)
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("insert verification document");
    }

    async fn finish(self) {
        let Room {
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

async fn call_multipart(
    room: &Room,
    who: Option<&Who>,
    uri: &str,
    boundary: &str,
    payload: Vec<u8>,
    idempotency_key: Option<&str>,
) -> Answer {
    let mut state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    state.document_store = room.store.clone();
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default().method(Method::POST).uri(uri);
    req = req.insert_header((header::HOST, HOST));

    if let Some(w) = who {
        req = req.insert_header((AS, w.spec()));
        let session_cookie = format!("__Host-xc_session={}", w.session);
        req = req.insert_header((header::COOKIE, session_cookie));
    }

    req = req
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header((
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        ));

    if let Some(key) = idempotency_key {
        req = req.insert_header(("idempotency-key", key.to_owned()));
    }

    req = req.set_payload(payload);

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let parsed_body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    Answer {
        status,
        headers,
        text,
        body: parsed_body,
        bytes: bytes.to_vec(),
    }
}

// -----------------------------------------------------------------------------
// D1: CX_2 -> CX_1's case, CX_1's key, the same bytes
// Required result: 403 outOfScope, no row (main: 200 with CX_1's document)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn d1_cx2_uploads_to_cx1_case_with_cx1_key_and_same_bytes_refused_out_of_scope() {
    let room = Room::start().await;
    let cust_b = Who::customer("user-cust-2", "CX_2");
    let case_a_id = Uuid::new_v4();
    let doc_a_id = Uuid::new_v4();
    let key = "IDEM-KEY-D1";

    room.insert_case(case_a_id, ORG_1, "CX_1", "DRAFT").await;

    let doc_ref = xc_kernel::document_store::document_ref(&case_a_id.to_string(), key).unwrap();
    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_a_id,
        ORG_1,
        case_a_id,
        "PASSPORT",
        Some(&doc_ref),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    let boundary = "---------------------------boundaryd1";
    let payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", SECRET_DOC_BYTES)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust_b),
        &format!("/api/app/verification/cases/{case_a_id}/documents"),
        boundary,
        payload,
        Some(key),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::FORBIDDEN,
        "D1: CX_2 must be refused with 403 outOfScope: {}",
        ans.text
    );
    assert_eq!(ans.code(), "outOfScope");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM verification_documents WHERE case_id = $1")
            .bind(case_a_id)
            .fetch_one(&room.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);

    room.finish().await;
}

// -----------------------------------------------------------------------------
// D2: CX_2 -> CX_1's case, CX_1's key, different bytes
// Required result: 403 outOfScope (main: 409)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn d2_cx2_uploads_to_cx1_case_with_cx1_key_and_different_bytes_refused_out_of_scope() {
    let room = Room::start().await;
    let cust_b = Who::customer("user-cust-2", "CX_2");
    let case_a_id = Uuid::new_v4();
    let doc_a_id = Uuid::new_v4();
    let key = "IDEM-KEY-D2";

    room.insert_case(case_a_id, ORG_1, "CX_1", "DRAFT").await;

    let doc_ref = xc_kernel::document_store::document_ref(&case_a_id.to_string(), key).unwrap();
    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_a_id,
        ORG_1,
        case_a_id,
        "PASSPORT",
        Some(&doc_ref),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    let boundary = "---------------------------boundaryd2";
    let payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", DIFFERENT_DOC_BYTES)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust_b),
        &format!("/api/app/verification/cases/{case_a_id}/documents"),
        boundary,
        payload,
        Some(key),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::FORBIDDEN,
        "D2: CX_2 must be refused with 403 outOfScope: {}",
        ans.text
    );
    assert_eq!(ans.code(), "outOfScope");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// D3: CX_1 replays its own key with the same bytes
// Required result: 200, the same document (kept)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn d3_cx1_replays_own_key_with_same_bytes_returns_existing_document() {
    let room = Room::start().await;
    let cust_a = Who::customer("user-cust-1", "CX_1");
    let case_a_id = Uuid::new_v4();
    let doc_a_id = Uuid::new_v4();
    let key = "IDEM-KEY-D3";

    room.insert_case(case_a_id, ORG_1, "CX_1", "DRAFT").await;

    let doc_ref = xc_kernel::document_store::document_ref(&case_a_id.to_string(), key).unwrap();
    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_a_id,
        ORG_1,
        case_a_id,
        "PASSPORT",
        Some(&doc_ref),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    let boundary = "---------------------------boundaryd3";
    let payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", SECRET_DOC_BYTES)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust_a),
        &format!("/api/app/verification/cases/{case_a_id}/documents"),
        boundary,
        payload,
        Some(key),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "D3: CX_1 replay with same bytes must return 200: {}",
        ans.text
    );
    assert_eq!(ans.body["id"], doc_a_id.to_string());

    room.finish().await;
}

// -----------------------------------------------------------------------------
// D4: CX_1's own key with different bytes
// Required result: 409 idempotencyConflict (kept)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn d4_cx1_replays_own_key_with_different_bytes_returns_idempotency_conflict() {
    let room = Room::start().await;
    let cust_a = Who::customer("user-cust-1", "CX_1");
    let case_a_id = Uuid::new_v4();
    let doc_a_id = Uuid::new_v4();
    let key = "IDEM-KEY-D4";

    room.insert_case(case_a_id, ORG_1, "CX_1", "DRAFT").await;

    let doc_ref = xc_kernel::document_store::document_ref(&case_a_id.to_string(), key).unwrap();
    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_a_id,
        ORG_1,
        case_a_id,
        "PASSPORT",
        Some(&doc_ref),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    let boundary = "---------------------------boundaryd4";
    let payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", DIFFERENT_DOC_BYTES)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust_a),
        &format!("/api/app/verification/cases/{case_a_id}/documents"),
        boundary,
        payload,
        Some(key),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::CONFLICT,
        "D4: CX_1 replay with different bytes must return 409: {}",
        ans.text
    );
    assert_eq!(ans.code(), "idempotencyConflict");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// D5: unknown case id
// Required result: 404 notFound (kept)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn d5_upload_to_unknown_case_id_returns_not_found() {
    let room = Room::start().await;
    let cust_a = Who::customer("user-cust-1", "CX_1");
    let unknown_case_id = Uuid::new_v4();
    let key = "IDEM-KEY-D5";

    let boundary = "---------------------------boundaryd5";
    let payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", SECRET_DOC_BYTES)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust_a),
        &format!("/api/app/verification/cases/{unknown_case_id}/documents"),
        boundary,
        payload,
        Some(key),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "D5: upload to unknown case must return 404: {}",
        ans.text
    );
    assert_eq!(ans.code(), "notFound");

    room.finish().await;
}
