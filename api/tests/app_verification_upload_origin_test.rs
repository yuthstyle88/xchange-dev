//! Ticket #2134 — same-origin protection on `POST /api/app/verification/cases/{id}/documents`
//! (`app_verification_upload::handle_upload`).
//!
//! Pins §5 behaviour rows:
//! - O1: marker `1` + `Origin: https://evil.example` -> 400 `originRejected`; nothing stored
//! - O2: no marker + own Origin -> 400 `originRejected`; nothing stored
//! - O3: marker `1` + no Origin -> 400 `originRejected`; nothing stored
//! - O4: marker `true` + own Origin -> 400 `originRejected`; nothing stored
//! - O5: cross-origin and no `Idempotency-Key` -> 400 `originRejected` (not `validationFailed`); nothing stored
//! - K1: marker `1` + own Origin -> 200, document stored

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
use serde_json::Value;
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

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const FOREIGN_ORIGIN: &str = "https://evil.example";

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const USER_CUST_1: &str = "55555555-5555-5555-5555-555555555555";

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
            "{}|{}|{}|{}|{}|{}",
            self.user_id,
            self.role.as_str(),
            self.branch.as_deref().unwrap_or(""),
            self.customer_id.as_deref().unwrap_or(""),
            self.org_id,
            self.session
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

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

struct Room {
    pool: PgPool,
    schema: String,
    store_dir: std::path::PathBuf,
    store: Arc<CountingStore>,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let schema = unique("c10_origin");
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

        let store_dir = std::env::temp_dir().join(format!("xc-docstore-origin-{}", Uuid::new_v4()));
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
            "INSERT INTO users (id, organization_id, name, status, created_by, created_at) VALUES \
             ($1, $2, 'Customer Linked User', 'ACTIVE', 'seed', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed user");

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) VALUES \
             ('CX_1', $1, 'CX_1', 'Somchai Prasert', 'LIVE', now(), $2) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed customer CX_1 with linked user_id");
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

/// Calls `POST /api/app/verification/cases/{id}/documents` for a signed-in customer on their own
/// DRAFT case with a valid multipart body, applying only the specified `origin_headers` and optional
/// `idempotency_key`.
async fn call_upload_with_headers(
    room: &Room,
    origin_headers: &[(&str, &str)],
    idempotency_key: Option<&str>,
) -> Answer {
    let cust = Who::customer(USER_CUST_1, "CX_1");
    let case_id = Uuid::new_v4();
    room.insert_draft_case(case_id, "CX_1").await;

    let boundary = "---------------------------origincheckboundary";
    let png = valid_png_bytes();
    let payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", &png)),
    );

    let mut state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    state.document_store = room.store.clone();
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let uri = format!("/api/app/verification/cases/{case_id}/documents");
    let session_cookie = format!("__Host-xc_session={}", cust.session);
    let mut req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(&uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, cust.spec()))
        .insert_header((header::COOKIE, session_cookie))
        .insert_header((
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        ));

    if let Some(key) = idempotency_key {
        req = req.insert_header(("idempotency-key", key.to_owned()));
    }
    for (name, value) in origin_headers {
        req = req.insert_header((*name, *value));
    }

    req = req.set_payload(payload);

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    Answer { status, text, body }
}

fn assert_origin_rejected(answer: &Answer) {
    assert_eq!(
        (answer.status, answer.code()),
        (StatusCode::BAD_REQUEST, "originRejected"),
        "expected 400 originRejected, got {} {}",
        answer.status,
        answer.text
    );
}

async fn assert_nothing_stored(room: &Room) {
    assert_eq!(
        room.count("SELECT count(*) FROM verification_documents")
            .await,
        0,
        "no verification_documents row may be stored"
    );
    assert_eq!(
        room.store.puts.load(Ordering::SeqCst),
        0,
        "no DocumentStore::put call may occur"
    );
    assert_eq!(
        room.count("SELECT count(*) FROM idempotency_records").await,
        0,
        "no idempotency_records row may be stored"
    );
}

/// O1: marker `1` + `Origin: https://evil.example` -> 400 `originRejected`; nothing stored.
#[actix_web::test]
async fn o1_marker_1_with_foreign_origin_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let answer = call_upload_with_headers(
        &room,
        &[("x-xchange-request", "1"), ("origin", FOREIGN_ORIGIN)],
        Some(&unique("o1-key")),
    )
    .await;
    assert_origin_rejected(&answer);
    assert_nothing_stored(&room).await;
    room.finish().await;
}

/// O2: no marker + own Origin -> 400 `originRejected`; nothing stored.
#[actix_web::test]
async fn o2_no_marker_with_own_origin_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let answer =
        call_upload_with_headers(&room, &[("origin", SAME_ORIGIN)], Some(&unique("o2-key"))).await;
    assert_origin_rejected(&answer);
    assert_nothing_stored(&room).await;
    room.finish().await;
}

/// O3: marker `1` + no Origin -> 400 `originRejected`; nothing stored.
#[actix_web::test]
async fn o3_marker_1_with_no_origin_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let answer = call_upload_with_headers(
        &room,
        &[("x-xchange-request", "1")],
        Some(&unique("o3-key")),
    )
    .await;
    assert_origin_rejected(&answer);
    assert_nothing_stored(&room).await;
    room.finish().await;
}

/// O4: marker `true` + own Origin -> 400 `originRejected`; nothing stored.
#[actix_web::test]
async fn o4_marker_true_with_own_origin_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let answer = call_upload_with_headers(
        &room,
        &[("x-xchange-request", "true"), ("origin", SAME_ORIGIN)],
        Some(&unique("o4-key")),
    )
    .await;
    assert_origin_rejected(&answer);
    assert_nothing_stored(&room).await;
    room.finish().await;
}

/// O5: cross-origin and no `Idempotency-Key` -> 400 `originRejected`, not `validationFailed`:
/// the origin check runs first.
#[actix_web::test]
async fn o5_cross_origin_without_idempotency_key_answers_origin_rejected_first() {
    let room = Room::start().await;
    let answer = call_upload_with_headers(
        &room,
        &[("x-xchange-request", "1"), ("origin", FOREIGN_ORIGIN)],
        None,
    )
    .await;
    assert_origin_rejected(&answer);
    assert_nothing_stored(&room).await;
    room.finish().await;
}

/// K1: marker `1` + own Origin -> 200, document stored (unchanged).
#[actix_web::test]
async fn k1_marker_1_with_own_origin_stores_document() {
    let room = Room::start().await;
    let answer = call_upload_with_headers(
        &room,
        &[("x-xchange-request", "1"), ("origin", SAME_ORIGIN)],
        Some(&unique("k1-key")),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "body: {}", answer.text);
    assert_eq!(answer.body["documentType"], "PASSPORT");
    assert_eq!(
        room.count("SELECT count(*) FROM verification_documents")
            .await,
        1
    );
    assert_eq!(room.store.puts.load(Ordering::SeqCst), 1);
    assert_eq!(
        room.count("SELECT count(*) FROM idempotency_records").await,
        1
    );
    room.finish().await;
}
