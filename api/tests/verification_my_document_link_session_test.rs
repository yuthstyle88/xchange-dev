//! Customer document link (`verificationMyDocumentLink`) is bound to the caller's own session (#2006).
//!
//! Design `docs/design/blueprint/verification.md`:
//! - `:95` (V10): "The link is signed by the server, names one document and one actor, and expires";
//! - `:217` (§4.4): `POST /api/app/verification/documents/{id}/link` (`verificationMyDocumentLink`);
//! - `:235` (§4.5): `GET /api/verification/content/{token}` "serves one link; refuses an expired one or one
//!   used by another session".
//!
//! One customer `C` owns document `D` and has two live sessions: `S_old` (expires in 2 h) and `S_new`
//! (expires in 8 h). Every session is a real `user_sessions` row whose `hash` is `sha256_hex` of the cookie,
//! as `SessionService` writes it. Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::application::session::sha256_hex;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::document_store::DocumentStore;
use xc_kernel::pii::{PiiCipher, PiiKeys};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::document_store::FsDocumentStore;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const BRANCH_MAIN: &str = "MAIN";
const CUSTOMER_USER_C: &str = "55555555-5555-5555-5555-555555555555";
const CUSTOMER_ID_C: &str = "CX_1";
const DOC_BYTES: &[u8] =
    b"\x89PNG\r\n\x1a\nCUSTOMER_PASSPORT_IMAGE_BYTES_FOR_MY_DOC_LINK_SESSION_TEST";
const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// Customer `C` of `ORG_1` owning `CUSTOMER_ID_C`, identified by the `x-test-as` header.
fn customer_spec(user_id: &str, customer_id: &str) -> String {
    format!("{user_id}|{}|{customer_id}", Role::Customer.as_str())
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Box::pin(async move {
            let parts: Vec<&str> = spec.split('|').collect();
            let user = if parts.len() == 3 {
                let user_id = parts[0];
                let role_str = parts[1];
                let customer_id = parts[2];
                Role::parse(role_str).map(|r| {
                    let from = Utc::now() - ChronoDuration::days(30);
                    User::new(
                        user_id,
                        ORG_1,
                        vec![ScopedRole::new(r, None, from, None)],
                        Some(customer_id),
                    )
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

struct Answer {
    status: StatusCode,
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
        assert_ne!(options.get_port(), 6381, "port 6381 is the queue's");

        let schema = unique("vfmydocsess");
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

        common::install_test_pii_keys();
        let store_dir = std::env::temp_dir().join(format!("xc-docstore-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&store_dir).expect("create temp docstore dir");
        let keys = Arc::new(PiiKeys::parse(TEST_KEYS).expect("valid test keys"));
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
        room.seed().await;
        room
    }

    async fn seed(&self) {
        let org = Uuid::from_str(ORG_1).unwrap();
        sqlx::query(
            "INSERT INTO branches (id, organization_id, name) VALUES ($1, $2, 'Main Branch') \
             ON CONFLICT (id) DO UPDATE SET organization_id = EXCLUDED.organization_id",
        )
        .bind(BRANCH_MAIN)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("seed branch");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($2, $1, $2, 'Somchai Prasert', 'LIVE', now()) ON CONFLICT DO NOTHING",
        )
        .bind(org)
        .bind(CUSTOMER_ID_C)
        .execute(&self.pool)
        .await
        .expect("seed customer");
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ($2::uuid, $1, 'Customer C', 'ACTIVE', 'seed') ON CONFLICT (id) DO NOTHING",
        )
        .bind(org)
        .bind(CUSTOMER_USER_C)
        .execute(&self.pool)
        .await
        .expect("seed customer user");
    }

    /// A real signed-in session of `user_id`: the browser holds `cookie`, the server stores its
    /// `sha256_hex`, as `SessionService` does. Returns the cookie.
    async fn session(&self, user_id: &str, expires_in_hours: i64) -> String {
        let cookie = format!("tok-{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
             VALUES ($1, $2::uuid, 1, now() + make_interval(hours => $3::int), $4)",
        )
        .bind(sha256_hex(&cookie))
        .bind(user_id)
        .bind(expires_in_hours as i32)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("insert user_sessions row");
        cookie
    }

    /// A case owned by `CUSTOMER_ID_C` at MAIN with one stored PASSPORT document; returns the document id.
    async fn document(&self) -> Uuid {
        let case_id = Uuid::new_v4();
        let doc_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO verification_cases \
             (id, organization_id, customer_id, provider, external_case_ref, status, policy_version, branch_id, \
              submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'STAFF', NULL, 'UNDER_REVIEW', 1, $4, now(), NULL, NULL, NULL, NULL, 1, now(), now())",
        )
        .bind(case_id)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(CUSTOMER_ID_C)
        .bind(BRANCH_MAIN)
        .execute(&self.pool)
        .await
        .expect("insert case");
        let object_ref = format!("session-test-{doc_id}");
        self.store
            .put(&object_ref, DOC_BYTES)
            .await
            .expect("store document bytes");
        sqlx::query(
            "INSERT INTO verification_documents \
             (id, organization_id, case_id, document_type, encrypted_object_ref, key_id, checksum, mime, size_bytes, \
              uploaded_at, uploaded_by, retention_until, access_class, removed_at, purged_at, created_at) \
             VALUES ($1, $2, $3, 'PASSPORT', $4, 'test-1', $5, 'image/png', $6, now(), $7::uuid, \
                     now() + interval '365 days', 'IDENTITY_DOCUMENT', NULL, NULL, now())",
        )
        .bind(doc_id)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(case_id)
        .bind(&object_ref)
        .bind(&Sha256::digest(DOC_BYTES)[..])
        .bind(DOC_BYTES.len() as i32)
        .bind(CUSTOMER_USER_C)
        .execute(&self.pool)
        .await
        .expect("insert document");
        doc_id
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

/// One request as customer `user_id` carrying `cookie` as the session cookie.
async fn call(
    room: &Room,
    user_id: &str,
    cookie: &str,
    method: Method,
    uri: &str,
    body: Option<(Value, &str)>,
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
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, customer_spec(user_id, CUSTOMER_ID_C)))
        .insert_header((header::COOKIE, format!("__Host-xc_session={cookie}")));
    if let Some((json_body, key)) = body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(json_body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        bytes: bytes.to_vec(),
    }
}

/// The content URL the customer link command (`POST /api/app/verification/documents/{id}/link`)
/// issues to `user_id` on `cookie`.
async fn link(room: &Room, user_id: &str, cookie: &str, doc_id: Uuid) -> String {
    let ans = call(
        room,
        user_id,
        cookie,
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/link"),
        Some((json!({}), &unique("IDEM-MY-DOC-LINK"))),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "issue customer link: {}",
        ans.text
    );
    ans.body["url"]
        .as_str()
        .expect("the link answer carries url")
        .to_owned()
}

// Row 1: link minted with S_old's cookie, opened with S_old's cookie -> 200, D's bytes.
#[actix_web::test]
async fn row_1_link_minted_with_older_session_opens_with_older_session() {
    let room = Room::start().await;
    let doc = room.document().await;
    let s_old = room.session(CUSTOMER_USER_C, 2).await;
    let _s_new = room.session(CUSTOMER_USER_C, 8).await;

    let url = link(&room, CUSTOMER_USER_C, &s_old, doc).await;
    let ans = call(&room, CUSTOMER_USER_C, &s_old, Method::GET, &url, None).await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "Row 1: link minted with S_old must open with S_old: {}",
        ans.text
    );
    assert_eq!(ans.bytes, DOC_BYTES, "the document's own bytes");
    room.finish().await;
}

// Row 2: link minted with S_old's cookie, opened with S_new's cookie -> 403 linkRefused.
#[actix_web::test]
async fn row_2_link_minted_with_older_session_is_refused_with_newer_session() {
    let room = Room::start().await;
    let doc = room.document().await;
    let s_old = room.session(CUSTOMER_USER_C, 2).await;
    let s_new = room.session(CUSTOMER_USER_C, 8).await;

    let url = link(&room, CUSTOMER_USER_C, &s_old, doc).await;
    let ans = call(&room, CUSTOMER_USER_C, &s_new, Method::GET, &url, None).await;

    assert_eq!(ans.status, StatusCode::FORBIDDEN, "{}", ans.text);
    assert_eq!(ans.code(), "linkRefused", "§4.5: used by another session");
    room.finish().await;
}

// Row 3: link minted with S_new's cookie, opened with S_new's cookie -> 200, D's bytes.
#[actix_web::test]
async fn row_3_link_minted_with_newer_session_opens_with_newer_session() {
    let room = Room::start().await;
    let doc = room.document().await;
    let _s_old = room.session(CUSTOMER_USER_C, 2).await;
    let s_new = room.session(CUSTOMER_USER_C, 8).await;

    let url = link(&room, CUSTOMER_USER_C, &s_new, doc).await;
    let ans = call(&room, CUSTOMER_USER_C, &s_new, Method::GET, &url, None).await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "Row 3: link minted with S_new must open with S_new: {}",
        ans.text
    );
    assert_eq!(ans.bytes, DOC_BYTES, "the document's own bytes");
    room.finish().await;
}

// Row 4: link minted with S_new's cookie, opened with S_old's cookie -> 403 linkRefused.
#[actix_web::test]
async fn row_4_link_minted_with_newer_session_is_refused_with_older_session() {
    let room = Room::start().await;
    let doc = room.document().await;
    let s_old = room.session(CUSTOMER_USER_C, 2).await;
    let s_new = room.session(CUSTOMER_USER_C, 8).await;

    let url = link(&room, CUSTOMER_USER_C, &s_new, doc).await;
    let ans = call(&room, CUSTOMER_USER_C, &s_old, Method::GET, &url, None).await;

    assert_eq!(ans.status, StatusCode::FORBIDDEN, "{}", ans.text);
    assert_eq!(ans.code(), "linkRefused", "§4.5: used by another session");
    room.finish().await;
}
