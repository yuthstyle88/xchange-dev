//! Ticket #2576: Removing an already-removed verification document answers 404 notFound,
//! leaves removed_at unchanged, and writes no second audit event.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

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
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
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
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

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

    fn ok(&self, what: &str) -> &Value {
        assert_eq!(self.status, StatusCode::OK, "{what}: {}", self.text);
        &self.body
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

        let schema = unique("c10_remove_twice");
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

async fn call(
    room: &Room,
    who: Option<&Who>,
    method: Method,
    uri: &str,
    body: Option<(Value, &str)>, // (payload, idempotency_key)
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

    let mut req = atest::TestRequest::default().method(method).uri(uri);
    req = req.insert_header((header::HOST, HOST));

    if let Some(w) = who {
        req = req.insert_header((AS, w.spec()));
        let session_cookie = format!("__Host-xc_session={}", w.session);
        req = req.insert_header((header::COOKIE, session_cookie));
    }

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

// ------------------------------------------------------------ behaviour table rows
// R1: remove a document, then remove it again with a new Idempotency-Key
// 2nd: 404 notFound; removed_at unchanged from the 1st; exactly one verification_document.remove
// row in audit_events for it (main: 200, removed_at rewritten, 2 events).
#[actix_web::test]
async fn r1_remove_document_twice_with_new_key_answers_404_and_leaves_audit_event_count_1() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(case_id, ORG_1, "CX_1", "DRAFT").await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-r1"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    // 1st remove
    let ans1 = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-1")),
    )
    .await;
    ans1.ok("1st remove succeeds");

    let first_removed_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT removed_at FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch removed_at after first remove");

    // 2nd remove with a new Idempotency-Key
    let ans2 = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-2")),
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::NOT_FOUND,
        "2nd remove must answer 404 notFound, got {}: {}",
        ans2.status,
        ans2.text
    );
    assert_eq!(ans2.code(), "notFound");

    let second_removed_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT removed_at FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch removed_at after second remove");
    assert_eq!(
        first_removed_at, second_removed_at,
        "removed_at must be unchanged after second remove"
    );

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events WHERE action = 'verification_document.remove' AND (after_redacted->>'document_id') = $1",
    )
    .bind(doc_id.to_string())
    .fetch_one(&room.pool)
    .await
    .expect("count audit events");
    assert_eq!(
        audit_count, 1,
        "exactly one audit event must be written for the document"
    );

    room.finish().await;
}

// R2: the 2nd remove repeats the 1st's Idempotency-Key and body
// the 1st answer replayed (200), as the command seam does today; no new event.
#[actix_web::test]
async fn r2_remove_document_replaying_idempotency_key_answers_200_and_no_new_event() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(case_id, ORG_1, "CX_1", "DRAFT").await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-r2"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    // 1st remove
    let ans1 = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-REPLAY")),
    )
    .await;
    ans1.ok("1st remove succeeds");

    // 2nd remove with the same Idempotency-Key and body
    let ans2 = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-REPLAY")),
    )
    .await;
    ans2.ok("2nd remove with same key is replayed as 200");

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events WHERE action = 'verification_document.remove' AND (after_redacted->>'document_id') = $1",
    )
    .bind(doc_id.to_string())
    .fetch_one(&room.pool)
    .await
    .expect("count audit events");
    assert_eq!(audit_count, 1, "replay must not write a new audit event");

    room.finish().await;
}

// R3: remove a document that is not removed
// 200, removed_at set (keep: document_remove_succeeds_in_draft_and_refuses_when_case_locked).
#[actix_web::test]
async fn r3_remove_document_not_removed_succeeds_and_sets_removed_at() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(case_id, ORG_1, "CX_1", "DRAFT").await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-r3"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    let ans = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-R3")),
    )
    .await;
    ans.ok("remove document succeeds");

    let removed_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT removed_at FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch removed_at");
    assert!(removed_at.is_some(), "removed_at must be set");

    room.finish().await;
}

// R4: remove a document of another customer's case
// 403 outOfScope (keep: c10_routes_refuse_for_another_customers_case_and_document).
#[actix_web::test]
async fn r4_remove_document_of_another_customer_answers_403_out_of_scope() {
    let room = Room::start().await;
    let cust_2 = Who::customer("user-cust-2", "CX_2");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    // Case belongs to CX_1
    room.insert_case(case_id, ORG_1, "CX_1", "DRAFT").await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-r4"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
    )
    .await;

    // CX_2 tries to remove CX_1's document
    let ans = call(
        &room,
        Some(&cust_2),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-R4")),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::FORBIDDEN,
        "must answer 403 outOfScope: {}",
        ans.text
    );
    assert_eq!(ans.code(), "outOfScope");

    let removed_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT removed_at FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch removed_at");
    assert!(
        removed_at.is_none(),
        "document must not be removed by another customer"
    );

    room.finish().await;
}
