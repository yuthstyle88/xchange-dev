//! #1943: a staff document link is bound to the session that minted it, and the step from cookie to session hash is
//! made in one place (`caller_session_hash`, beside `identity_session` in `presentation/extractors/actor.rs`).
//! Written before the implementation by the ticket author (claude/c4d72c), on the BE lane controller claude/7ca80c's
//! order of 2026-10-15. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/customer-app.md` §9.2 (`Caller.session_hash`), `verification.md:95` (V10: the link
//! names one document and one actor and the session that minted it) and `:235` (§4.5: the content route refuses a
//! link "used by another session").
//!
//! On main (after #1942) the link command binds the link to `SELECT hash FROM user_sessions WHERE user_id = … ORDER BY
//! expires DESC LIMIT 1`, which is any live session of the user, not the caller's. With no live session it falls back
//! to `sha256_hex("session-{user_id}")`, and the content route hashes the cookie itself without asking whether the
//! session is live.
//!
//! Every session here is a real `user_sessions` row whose `hash` is `sha256_hex` of the cookie.

mod common;

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
use std::future::Future;
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
const REVIEWER_A: &str = "11111111-1111-1111-1111-111111111111";
const REVIEWER_B: &str = "22222222-2222-2222-2222-222222222222";
const DOC_BYTES: &[u8] = b"PASSPORT_IMAGE_BYTES_FOR_THE_SESSION_TEST";
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

/// An organisation-wide VERIFICATION_REVIEWER of ORG_1, identified by the `x-test-as` header.
fn reviewer_spec(user_id: &str) -> String {
    format!("{user_id}|{}", Role::VerificationReviewer.as_str())
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
            let user = spec.split_once('|').and_then(|(user_id, role)| {
                Role::parse(role).map(|r| {
                    let from = Utc::now() - ChronoDuration::days(30);
                    User::new(
                        user_id,
                        ORG_1,
                        vec![ScopedRole::new(r, None, from, None)],
                        None,
                    )
                })
            });
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

        let schema = unique("vfcaller");
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
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ($2::uuid, $1, 'Reviewer A', 'ACTIVE', 'seed'), \
             ($3::uuid, $1, 'Reviewer B', 'ACTIVE', 'seed') ON CONFLICT (id) DO NOTHING",
        )
        .bind(org)
        .bind(REVIEWER_A)
        .bind(REVIEWER_B)
        .execute(&self.pool)
        .await
        .expect("seed users");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CX_1', $1, 'CX_1', 'Somchai Prasert', 'LIVE', now()) ON CONFLICT DO NOTHING",
        )
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("seed customer");
    }

    /// An UNDER_REVIEW case at MAIN with one stored PASSPORT document; returns the document id.
    async fn document(&self) -> Uuid {
        let case_id = Uuid::new_v4();
        let doc_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO verification_cases \
             (id, organization_id, customer_id, provider, external_case_ref, status, policy_version, branch_id, \
              submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at, updated_at) \
             VALUES ($1, $2, 'CX_1', 'STAFF', NULL, 'UNDER_REVIEW', 1, $3, now(), NULL, NULL, NULL, NULL, 1, now(), now())",
        )
        .bind(case_id)
        .bind(Uuid::from_str(ORG_1).unwrap())
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
        .bind(REVIEWER_A)
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

/// One request as `user_id` (a reviewer) carrying `cookie` as the session cookie.
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
        .insert_header((AS, reviewer_spec(user_id)))
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

/// The content URL the link command issues to `user_id` on `cookie`.
async fn link(room: &Room, user_id: &str, cookie: &str, doc_id: Uuid) -> String {
    let ans = call(
        room,
        user_id,
        cookie,
        Method::POST,
        &format!("/api/operations/verification/documents/{doc_id}/link"),
        Some((json!({"expectedVersion": 1}), &unique("IDEM-LINK"))),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "issue link: {}", ans.text);
    ans.body["url"]
        .as_str()
        .expect("the link answer carries url")
        .to_owned()
}

/// A live session of `user_id` that expires `hours` from now (negative: already expired). Returns the cookie.
async fn session_expiring(room: &Room, user_id: &str, hours: i64) -> String {
    let cookie = format!("tok-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
         VALUES ($1, $2::uuid, 1, now() + make_interval(hours => $3::int), $4)",
    )
    .bind(sha256_hex(&cookie))
    .bind(user_id)
    .bind(hours as i32)
    .bind(Uuid::from_str(ORG_1).unwrap())
    .execute(&room.pool)
    .await
    .expect("insert user_sessions row");
    cookie
}

async fn link_answer(room: &Room, user_id: &str, cookie: &str, doc_id: Uuid) -> Answer {
    call(
        room,
        user_id,
        cookie,
        Method::POST,
        &format!("/api/operations/verification/documents/{doc_id}/link"),
        Some((json!({"expectedVersion": 1}), &unique("IDEM-LINK"))),
    )
    .await
}

#[actix_web::test]
async fn a_link_opens_only_on_the_session_that_minted_it() {
    let room = Room::start().await;
    let doc = room.document().await;
    // the session that mints expires first: "the user's latest session" is the other one
    let minting = session_expiring(&room, REVIEWER_A, 2).await;
    let other_device = session_expiring(&room, REVIEWER_A, 8).await;

    let url = link(&room, REVIEWER_A, &minting, doc).await;
    let mine = call(&room, REVIEWER_A, &minting, Method::GET, &url, None).await;
    assert_eq!(
        mine.status,
        StatusCode::OK,
        "the minting session opens it: {}",
        mine.text
    );
    assert_eq!(mine.bytes, DOC_BYTES);

    let other = call(&room, REVIEWER_A, &other_device, Method::GET, &url, None).await;
    assert_eq!(other.status, StatusCode::FORBIDDEN, "{}", other.text);
    assert_eq!(
        other.code(),
        "linkRefused",
        "the same reviewer on another session"
    );
    room.finish().await;
}

#[actix_web::test]
async fn with_no_live_session_the_link_command_refuses_and_mints_nothing() {
    let room = Room::start().await;
    let doc = room.document().await;
    let expired = session_expiring(&room, REVIEWER_A, -1).await;
    for (cookie, what) in [
        ("not-a-session".to_owned(), "a cookie that is no session"),
        (expired, "an expired session"),
        (format!("session-{REVIEWER_A}"), "the old fallback's cookie"),
    ] {
        let ans = link_answer(&room, REVIEWER_A, &cookie, doc).await;
        assert_eq!(ans.status, StatusCode::FORBIDDEN, "{what}: {}", ans.text);
        assert_eq!(ans.code(), "linkRefused", "{what}");
        assert!(
            ans.body.get("url").is_none(),
            "{what}: no link is handed out"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_session_that_expires_after_minting_no_longer_opens_the_link() {
    let room = Room::start().await;
    let doc = room.document().await;
    let cookie = session_expiring(&room, REVIEWER_A, 8).await;
    let url = link(&room, REVIEWER_A, &cookie, doc).await;
    sqlx::query("UPDATE user_sessions SET expires = now() - interval '1 minute' WHERE hash = $1")
        .bind(sha256_hex(&cookie))
        .execute(&room.pool)
        .await
        .expect("expire the session");

    let ans = call(&room, REVIEWER_A, &cookie, Method::GET, &url, None).await;
    assert_eq!(ans.status, StatusCode::FORBIDDEN, "{}", ans.text);
    assert_eq!(ans.code(), "linkRefused");
    room.finish().await;
}

/// The step from cookie to session hash is made in one place: the link command and the content route call
/// `caller_session_hash` and hash nothing themselves, and nothing falls back to a made-up session.
#[test]
fn the_cookie_is_hashed_in_one_place_and_nothing_falls_back() {
    let src = |rel: &str| {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
    };
    let actor = src("src/presentation/extractors/actor.rs");
    assert!(
        actor.contains("pub async fn caller_session_hash("),
        "extractors/actor.rs defines caller_session_hash beside identity_session"
    );
    for rel in [
        "src/presentation/http/v3/commands/verification_document_link.rs",
        "src/presentation/http/verification_content.rs",
    ] {
        let text = src(rel);
        assert!(!text.contains("sha256_hex("), "{rel} hashes nothing itself");
        assert!(
            !text.contains("session-{"),
            "{rel} names no made-up session"
        );
        assert!(
            !text.contains("ORDER BY expires DESC"),
            "{rel} picks no session of the user's own choosing"
        );
    }
    assert!(
        src("src/presentation/http/verification_content.rs").contains("caller_session_hash"),
        "the content route asks the helper"
    );
}
