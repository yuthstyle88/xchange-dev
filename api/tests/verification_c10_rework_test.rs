//! Rework tests for VF-8 (#1399, PR #1879):
//! 1. `app_verification_upload` delegates to `CaseService::add_document`:
//!    - judges file type by content bytes only (refuses non-PNG/JPEG/PDF bytes even when
//!      multipart `Content-Type` header says `image/png`, with `validationFailed` `"ชนิดไฟล์ไม่รองรับ"`);
//!    - refuses empty file (`0` bytes) with `validationFailed` `"ไฟล์ใหญ่เกิน 15 MB"`;
//!    - records `uploaded_by` as the customer's own linked `customers.user_id` (not the oldest user in the org);
//!    - writes audit row with `action = 'verification_document.add'`, `scope = org_id`,
//!      `object_ref = case_id`, `actor_id = customer_id`.
//! 2. `app_verification_upload::ACTION_NAME` is `"verificationDocumentUpload"` (gated by its own
//!    row in `xc_auth::domain::policy::areas::verification_self`, not `"verificationCaseOpen"`).
//! 3. Body exceeding payload limit answers 400 `validationFailed` with `"ไฟล์ใหญ่เกิน 15 MB"`.
//! 4. Upload idempotency uses `PgIdempotencyStore` (`idempotency_records`):
//!    - replaying the same `Idempotency-Key` with the same `(case_id, documentType, file)` returns
//!      the cached 200 OK response;
//!    - replaying the same `Idempotency-Key` with the same file bytes but a different `documentType`
//!      returns 409 `idempotencyConflict`.

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
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::areas::verification_self;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{PiiCipher, PiiKeys};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::document_store::FsDocumentStore;
use xchange::presentation::http::app_verification_upload;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const USER_OLDEST: &str = "11111111-1111-1111-1111-111111111111";
const USER_CUST_1: &str = "55555555-5555-5555-5555-555555555555";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
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
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let schema = unique("c10_rework");
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

        let store_dir = std::env::temp_dir().join(format!("xc-docstore-rework-{}", Uuid::new_v4()));
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

        // Seed oldest user first, then customer's linked user second
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by, created_at) VALUES \
             ($1, $3, 'Oldest Org User', 'ACTIVE', 'seed', now() - interval '10 days'), \
             ($2, $3, 'Customer Linked User', 'ACTIVE', 'seed', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::from_str(USER_OLDEST).unwrap())
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed users");

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
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let parsed_body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    Answer {
        status,
        text,
        body: parsed_body,
    }
}

// -----------------------------------------------------------------------------
// Fix 2 proof: ACTION_NAME is "verificationDocumentUpload" and matches policy row
// -----------------------------------------------------------------------------
#[test]
fn upload_route_action_name_is_verification_document_upload_and_gated_in_policy() {
    assert_eq!(
        app_verification_upload::ACTION_NAME,
        "verificationDocumentUpload",
        "upload route must authorize against verificationDocumentUpload, not verificationCaseOpen"
    );
    let has_row = verification_self::ROWS
        .iter()
        .any(|r| r.action == app_verification_upload::ACTION_NAME);
    assert!(
        has_row,
        "areas::verification_self::ROWS must contain a row for ACTION_NAME"
    );
}

// -----------------------------------------------------------------------------
// Fix 1 proof: Content-only MIME check, empty file refusal, linked uploaded_by,
// and CaseService::add_document audit row shape
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn upload_route_delegates_to_case_service_add_document() {
    let room = Room::start().await;
    // Note: session user_id is non-UUID ("session-user-cx1") so the old raw-SQL fallback
    // would have picked USER_OLDEST instead of CX_1's linked USER_CUST_1.
    let cust = Who::customer("session-user-cx1", "CX_1");
    let case_id = Uuid::new_v4();
    room.insert_draft_case(case_id, "CX_1").await;

    let boundary = "---------------------------reworkfix1boundary";

    // (1a) Non-PNG/JPEG/PDF bytes with Content-Type: image/png must be refused by content check
    let spoofed_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some((
            "file",
            "spoof.png",
            "image/png",
            b"NOT_A_REAL_PNG_OR_JPEG_OR_PDF",
        )),
    );
    let ans_spoof = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        spoofed_payload,
        Some("IDEM-REWORK-SPOOF"),
    )
    .await;
    assert_eq!(
        ans_spoof.status,
        StatusCode::BAD_REQUEST,
        "spoofed Content-Type with non-image bytes must be refused with 400: {}",
        ans_spoof.text
    );
    assert_eq!(ans_spoof.code(), "validationFailed");
    assert_eq!(
        ans_spoof.body["error"]["message"].as_str().unwrap_or(""),
        "ชนิดไฟล์ไม่รองรับ"
    );

    // (1b) Empty file (0 bytes) must be refused with TOO_LARGE sentence
    let empty_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "empty.png", "image/png", b"")),
    );
    let ans_empty = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        empty_payload,
        Some("IDEM-REWORK-EMPTY"),
    )
    .await;
    assert_eq!(
        ans_empty.status,
        StatusCode::BAD_REQUEST,
        "empty file must be refused with 400: {}",
        ans_empty.text
    );
    assert_eq!(ans_empty.code(), "validationFailed");
    assert_eq!(
        ans_empty.body["error"]["message"].as_str().unwrap_or(""),
        "ไฟล์ใหญ่เกิน 15 MB"
    );

    // (1d & 1e) Valid PNG upload records customer's linked user_id and CaseService audit shape
    let png = valid_png_bytes();
    let valid_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", &png)),
    );
    let ans_ok = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        valid_payload,
        Some("IDEM-REWORK-VALID"),
    )
    .await;
    let doc_json = ans_ok.ok("valid PNG upload");
    let doc_id = Uuid::from_str(doc_json["id"].as_str().expect("doc id")).expect("doc uuid");

    let uploaded_by: Uuid =
        sqlx::query_scalar("SELECT uploaded_by FROM verification_documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&room.pool)
            .await
            .expect("query uploaded_by");
    assert_eq!(
        uploaded_by,
        Uuid::from_str(USER_CUST_1).unwrap(),
        "uploaded_by must be the customer's own linked user_id (USER_CUST_1), not USER_OLDEST"
    );

    let (actor_id, actor_role, scope, action, object_ref, after_redacted): (
        String,
        String,
        String,
        String,
        String,
        Value,
    ) = sqlx::query_as(
        "SELECT actor_id, actor_role, scope, action, object_ref, after_redacted \
         FROM audit_events WHERE action = 'verification_document.add' AND object_ref = $1",
    )
    .bind(case_id.to_string())
    .fetch_one(&room.pool)
    .await
    .expect("CaseService::add_document must write verification_document.add audit event");

    assert_eq!(actor_id, "CX_1");
    assert_eq!(actor_role, "CUSTOMER");
    assert_eq!(scope, ORG_1);
    assert_eq!(action, "verification_document.add");
    assert_eq!(object_ref, case_id.to_string());
    assert_eq!(
        after_redacted["document_id"].as_str().unwrap_or(""),
        doc_id.to_string()
    );
    assert_eq!(
        after_redacted["document_type"].as_str().unwrap_or(""),
        "PASSPORT"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Fix 3 & Fix 4 proof: Body > 25 MB limit returns TOO_LARGE sentence, and
// upload idempotency goes through IdempotencyStore (same file + different
// documentType -> 409 idempotencyConflict)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn upload_idempotency_uses_idempotency_store_and_oversized_body_returns_too_large() {
    let room = Room::start().await;
    let cust = Who::customer("session-user-cx1", "CX_1");
    let case_id = Uuid::new_v4();
    room.insert_draft_case(case_id, "CX_1").await;

    let boundary = "---------------------------reworkidemboundary";
    let png = valid_png_bytes();

    let payload_passport = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "doc.png", "image/png", &png)),
    );
    let payload_nat_id = make_multipart(
        boundary,
        &[("documentType", "NATIONAL_ID")],
        Some(("file", "doc.png", "image/png", &png)),
    );

    let key = "IDEM-UPLOAD-V14-KEY";

    // 1. First upload succeeds
    let ans_1 = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        payload_passport.clone(),
        Some(key),
    )
    .await;
    let data_1 = ans_1.ok("first upload").clone();

    // Verify record written in idempotency_records
    let idem_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM idempotency_records WHERE operation = 'verificationDocumentUpload' AND key = $1 AND status = 'FINISHED'",
    )
    .bind(key)
    .fetch_one(&room.pool)
    .await
    .expect("count idempotency_records");
    assert_eq!(
        idem_count, 1,
        "upload must record FINISHED row in idempotency_records"
    );

    // 2. Replay with identical key, documentType, and file bytes returns cached answer
    let ans_replay = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        payload_passport,
        Some(key),
    )
    .await;
    let data_replay = ans_replay.ok("replay upload").clone();
    assert_eq!(data_1, data_replay);

    // 3. Same key + same file bytes + DIFFERENT documentType -> 409 idempotencyConflict
    let ans_conflict = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        payload_nat_id,
        Some(key),
    )
    .await;
    assert_eq!(
        ans_conflict.status,
        StatusCode::CONFLICT,
        "same key with different documentType must return 409: {}",
        ans_conflict.text
    );
    assert_eq!(ans_conflict.code(), "idempotencyConflict");

    // 4. Body over 25 MB limit returns 400 validationFailed with TOO_LARGE sentence
    let over_25mb = vec![b'A'; 25 * 1024 * 1024 + 4096];
    let ans_huge = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        over_25mb,
        Some("IDEM-OVER-25MB"),
    )
    .await;
    assert_eq!(
        ans_huge.status,
        StatusCode::BAD_REQUEST,
        "body over 25MB limit must return 400: {}",
        ans_huge.text
    );
    assert_eq!(ans_huge.code(), "validationFailed");
    assert!(
        ans_huge.text.contains("ไฟล์ใหญ่เกิน 15 MB"),
        "body over 25MB must return TOO_LARGE sentence, got: {}",
        ans_huge.text
    );

    room.finish().await;
}
