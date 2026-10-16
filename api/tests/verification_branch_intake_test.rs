//! VF-12 (#1398) — HTTP integration tests for at-branch intake: `POST /api/operations/verification/branch-cases`.
//! Command `verificationBranchCaseOpen` (design `docs/design/blueprint/verification.md` V17, §4.3, §4.5, §5, §8, §9.1 row VF-12).
//!
//! Written before the implementation by the test writer (gemini/warut-5, on the BE lane controller
//! claude/ad05bf8d's order, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.
//! Each test works in a schema of its own.

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_auth::domain::access::actor::Actor as AccessActor;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::AppState;
use xchange::presentation::http::branch_verification_intake::{BranchCaseOpenAnswer, ROUTE};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

mod common;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "B01";
const HOST: &str = "desk.example";
const ORIGIN: &str = "https://desk.example";
const AS: &str = "x-test-signed-in-as";
const MARKER: &[u8] = b"VF12-HTTP-SECRET-BYTES-3b8c";
const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;
const MUST_FINISH: Duration = Duration::from_secs(60);

static NEXT: AtomicU64 = AtomicU64::new(0);
static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

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

fn invalid_bytes() -> Vec<u8> {
    let mut b = b"NOT_A_VALID_IMAGE_OR_PDF".to_vec();
    b.extend_from_slice(MARKER);
    b
}

fn to_base64_standard(bytes: &[u8]) -> String {
    to_base64(bytes, false)
}

fn to_base64_url_safe(bytes: &[u8]) -> String {
    to_base64(bytes, true)
}

fn to_base64(bytes: &[u8], url_safe: bool) -> String {
    let charset: &[u8; 64] = if url_safe {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = if i + 1 < bytes.len() {
            bytes[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < bytes.len() {
            bytes[i + 2] as u32
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(charset[((triple >> 18) & 0x3F) as usize] as char);
        out.push(charset[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(charset[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(charset[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

// ── SessionReader mock ──────────────────────────────────────────────────────

struct HeaderUsers;

fn user_of(spec: &str) -> Option<AccessActor> {
    if spec.is_empty() {
        return None;
    }
    let mut parts = spec.splitn(4, '|');
    let id = parts.next()?;
    let role = parts.next()?;
    let branch = parts.next()?;
    let org = parts.next().unwrap_or(ORG_1);

    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(AccessActor::new(
        id,
        org,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
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
            .unwrap_or("")
            .to_owned();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                owner: None,
                staff: None,
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ── multipart helper ────────────────────────────────────────────────────────

fn build_multipart(
    boundary: &str,
    customer_id: Option<&str>,
    document_type: Option<&str>,
    file: Option<(&str, &str, &[u8])>,
) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some(c) = customer_id {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"customerId\"\r\n\r\n{c}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some(d) = document_type {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"documentType\"\r\n\r\n{d}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some((field_name, filename, bytes)) = file {
        let mime = if filename.ends_with(".png") {
            "image/png"
        } else if filename.ends_with(".pdf") {
            "application/pdf"
        } else {
            "image/jpeg"
        };
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

// ── database & test harness ─────────────────────────────────────────────────

struct HttpAnswer {
    status: StatusCode,
    text: String,
    json: Value,
}

struct TestEnv {
    pool: PgPool,
    schema: String,
}

impl TestEnv {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at a test database ending in `_test`")
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("vf12_http");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to schema");

        {
            let _slot = STARTING.acquire().await.expect("slot");
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
            Migrator::new(dir.as_path())
                .await
                .unwrap_or_else(|e| panic!("read migrations: {e}"))
                .run(&pool)
                .await
                .unwrap_or_else(|e| panic!("run migrations: {e}"));
        }

        Self { pool, schema }
    }

    async fn finish(self) {
        let TestEnv { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop schema");
        pool.close().await;
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

    async fn customer(&self, org: &str) -> (String, Uuid) {
        let user = self.user(org).await;
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'TestCustomer', 'LIVE', now(), '{user}')"
        ))
        .await;
        (id, user)
    }

    async fn branch(&self, org: &str, branch_id: &str) {
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{branch_id}', '{branch_id}', '{org}'::uuid) \
             ON CONFLICT DO NOTHING"
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

    async fn call_intake(
        &self,
        who_spec: Option<&str>,
        idempotency_key: Option<&str>,
        boundary: &str,
        payload: Vec<u8>,
    ) -> HttpAnswer {
        let state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);

        let app = atest::init_service(
            App::new()
                .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;

        let mut req = atest::TestRequest::post()
            .uri(ROUTE)
            .insert_header((header::HOST, HOST))
            .insert_header(("origin", ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("x-forwarded-proto", "https"))
            .insert_header((
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(payload);

        if let Some(spec) = who_spec {
            req = req.insert_header((AS, spec));
        }
        if let Some(k) = idempotency_key {
            req = req.insert_header(("idempotency-key", k));
        }

        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        HttpAnswer { status, text, json }
    }
}

// ── HTTP integration tests ───────────────────────────────────────────────────

#[actix_web::test]
async fn unauthenticated_request_is_refused_unauthorized() {
    let env = TestEnv::start().await;
    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some("CX1"),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env.call_intake(None, Some("k1"), boundary, payload).await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    env.finish().await;
}

#[actix_web::test]
async fn roles_without_verification_review_are_refused_forbidden() {
    // #1398 done criteria 1: TELLER, BRANCH_MANAGER, FINANCE, EXECUTIVE refused
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    for role in ["TELLER", "BRANCH_MANAGER", "FINANCE", "EXECUTIVE"] {
        let user_id = env.user(ORG_1).await;
        let spec = format!("{user_id}|{role}|{B01}|{ORG_1}");
        let answer = env
            .call_intake(Some(&spec), Some("k1"), boundary, payload.clone())
            .await;
        assert_eq!(
            answer.status,
            StatusCode::FORBIDDEN,
            "role {role} must be forbidden"
        );
        let code = answer.json["error"]["code"]
            .as_str()
            .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
        assert!(
            code == "roleNotAllowedHere" || code == "roleNotAllowed",
            "expected role refusal code for {role}, got: {code}"
        );
    }

    env.finish().await;
}

#[actix_web::test]
async fn organisation_scoped_reviewer_is_refused_out_of_scope() {
    // Design V17, #1398: an organisation-scoped reviewer is refused outOfScope
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;

    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|*|{ORG_1}"); // branch is * (org-wide)

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "outOfScope");

    env.finish().await;
}

#[actix_web::test]
async fn missing_idempotency_key_header_is_refused_bad_request() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;

    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env.call_intake(Some(&spec), None, boundary, payload).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    env.finish().await;
}

#[actix_web::test]
async fn missing_multipart_fields_is_refused_bad_request() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";

    // 1. Missing customerId
    let p1 = build_multipart(
        boundary,
        None,
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );
    let a1 = env.call_intake(Some(&spec), Some("k1"), boundary, p1).await;
    assert_eq!(a1.status, StatusCode::BAD_REQUEST);

    // 2. Missing documentType
    let p2 = build_multipart(
        boundary,
        Some(&customer_id),
        None,
        Some(("file", "doc.png", &png())),
    );
    let a2 = env.call_intake(Some(&spec), Some("k2"), boundary, p2).await;
    assert_eq!(a2.status, StatusCode::BAD_REQUEST);

    // 3. Missing file
    let p3 = build_multipart(boundary, Some(&customer_id), Some("PASSPORT"), None);
    let a3 = env.call_intake(Some(&spec), Some("k3"), boundary, p3).await;
    assert_eq!(a3.status, StatusCode::BAD_REQUEST);

    env.finish().await;
}

#[actix_web::test]
async fn successful_intake_returns_metadata_only_and_no_image_bytes() {
    // Design V9, V17, #1398 done criteria 1:
    // branch reviewer opens, attaches and submits in one transaction;
    // case SUBMITTED, branch_id = reviewer's branch, provider STAFF;
    // response contains metadata only, no image bytes!
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let secret_ident = "PASSPORT-IDENT-123456789";
    env.exec(&format!(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, nationality, status) \
         VALUES (gen_random_uuid(), '{ORG_1}'::uuid, '{customer_id}', 'PASSPORT', '{secret_ident}'::bytea, 'k-ident', gen_random_uuid()::text::bytea, 'THA', 'ACTIVE')"
    ))
    .await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";
    let file_bytes = png();
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "passport.png", &file_bytes)),
    );

    let answer = env
        .call_intake(Some(&spec), Some("idem-http-1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::OK);

    // Parse typed response
    let parsed: BranchCaseOpenAnswer =
        serde_json::from_value(answer.json.clone()).expect("parse BranchCaseOpenAnswer");
    assert_eq!(parsed.case.status, "SUBMITTED");
    assert_eq!(parsed.case.branch_id.as_deref(), Some(B01));
    assert_eq!(parsed.case.provider, "STAFF");
    assert_eq!(parsed.case.policy_version, 1);
    assert_eq!(parsed.case.version, 1);
    assert_eq!(parsed.document.document_type, "PASSPORT");
    assert_eq!(parsed.document.mime, "image/png");
    assert_eq!(parsed.document.size_bytes as usize, file_bytes.len());

    // CRITICAL: response text must NOT leak image bytes or secret marker
    assert!(
        !answer.text.contains(std::str::from_utf8(MARKER).unwrap()),
        "response must never contain uploaded file bytes or secret marker"
    );
    assert!(
        !answer.text.contains("base64"),
        "response must not carry base64 image data"
    );

    // DB assertions
    assert_eq!(
        env.count("SELECT count(*) FROM verification_cases").await,
        1
    );
    assert_eq!(
        env.count("SELECT count(*) FROM verification_documents")
            .await,
        1
    );
    assert_eq!(env.count("SELECT count(*) FROM audit_events").await, 1);
    let (audit_json,): (String,) = sqlx::query_as(AssertSqlSafe(
        "SELECT row_to_json(a)::text FROM audit_events a".to_string(),
    ))
    .fetch_one(&env.pool)
    .await
    .expect("fetch audit row");
    let marker_str = std::str::from_utf8(MARKER).unwrap();
    let b64_std = to_base64_standard(&file_bytes);
    let b64_url = to_base64_url_safe(&file_bytes);
    let hex_lower: String = file_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hex_upper: String = file_bytes.iter().map(|b| format!("{b:02X}")).collect();
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

    env.finish().await;
}

#[actix_web::test]
async fn customer_with_existing_open_case_answers_400_case_locked() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let _existing = env.force_open_case(&customer_id, ORG_1, "SUBMITTED").await;

    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k-open-conflict"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "caseLocked");

    env.finish().await;
}

#[actix_web::test]
async fn branch_review_allowed_false_answers_400_branch_review_not_allowed() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    env.exec(&format!(
        "UPDATE verification_policies SET branch_review_allowed = false \
         WHERE organization_id = '{ORG_1}'::uuid AND version = 1"
    ))
    .await;

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "branchReviewNotAllowed");

    env.finish().await;
}

#[actix_web::test]
async fn customer_of_another_organisation_answers_403_out_of_scope() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;

    let org_2 = Uuid::new_v4().to_string();
    env.exec(&format!(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ('{org_2}'::uuid, '{}', 'Asia/Bangkok', now())",
        unique("org2")
    ))
    .await;
    let (foreign_customer_id, _) = env.customer(&org_2).await;

    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&foreign_customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "outOfScope");

    env.finish().await;
}

#[actix_web::test]
async fn reviewer_linked_to_customer_answers_403_out_of_scope() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;

    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let linked_customer_id = unique("CX_LINKED");
    env.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
         VALUES ('{linked_customer_id}', '{ORG_1}'::uuid, '{linked_customer_id}', 'Self', 'LIVE', now(), '{user_id}')"
    ))
    .await;

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&linked_customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "outOfScope");

    env.finish().await;
}

#[actix_web::test]
async fn unaccepted_document_type_phone_answers_400_document_type_not_accepted() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PHONE"),
        Some(("file", "doc.png", &png())),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "documentTypeNotAccepted");

    env.finish().await;
}

#[actix_web::test]
async fn file_too_large_answers_400_validation_failed() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let mut large = vec![0u8; MAX_UPLOAD_BYTES + 1];
    large[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &large)),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "validationFailed");

    env.finish().await;
}

#[actix_web::test]
async fn unsupported_content_answers_400_validation_failed() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.txt", &invalid_bytes())),
    );

    let answer = env
        .call_intake(Some(&spec), Some("k1"), boundary, payload)
        .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let code = answer.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| answer.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "validationFailed");

    env.finish().await;
}

#[actix_web::test]
async fn idempotency_replay_answers_200_with_same_case_and_conflict_answers_409() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");

    let boundary = "bound123";
    let bytes = png();
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "doc.png", &bytes)),
    );

    // 1. Initial success
    let a1 = env
        .call_intake(
            Some(&spec),
            Some("http-idem-key"),
            boundary,
            payload.clone(),
        )
        .await;
    assert_eq!(a1.status, StatusCode::OK);
    let p1: BranchCaseOpenAnswer = serde_json::from_value(a1.json).expect("p1");

    // 2. Replay with identical payload
    let a2 = env
        .call_intake(Some(&spec), Some("http-idem-key"), boundary, payload)
        .await;
    assert_eq!(a2.status, StatusCode::OK);
    let p2: BranchCaseOpenAnswer = serde_json::from_value(a2.json).expect("p2");
    assert_eq!(p1.case.id, p2.case.id);
    assert_eq!(p1.document.id, p2.document.id);

    assert_eq!(
        env.count("SELECT count(*) FROM verification_cases").await,
        1
    );
    assert_eq!(
        env.count("SELECT count(*) FROM verification_documents")
            .await,
        1
    );
    assert_eq!(env.count("SELECT count(*) FROM audit_events").await, 1);

    // 3. Replay with different payload → 409 Conflict
    let payload_conflict = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "diff.jpeg", &jpeg())),
    );
    let a3 = env
        .call_intake(
            Some(&spec),
            Some("http-idem-key"),
            boundary,
            payload_conflict,
        )
        .await;
    assert_eq!(a3.status, StatusCode::CONFLICT);
    let code = a3.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| a3.json["error"].as_str().unwrap_or(""));
    assert_eq!(code, "idempotencyConflict");

    env.finish().await;
}

// ── same-origin rule (added: the route skipped `same_origin`, origin.rs:33) ──
//
// Every other cookie-session write refuses a cross-site request with 400 `ORIGIN_REJECTED`
// before it reads or stores anything. These tests pin that for this route. They build their own
// request (the helper `call_intake` above is left as it is) and count every write the route can
// make: case rows, document rows, audit rows, and `DocumentStore::put` calls.

const FOREIGN_ORIGIN: &str = "https://evil.example";

/// Wraps the state's real store and counts `put` calls.
struct CountingStore {
    inner: Arc<dyn xc_kernel::document_store::DocumentStore>,
    puts: AtomicU64,
}

type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

impl xc_kernel::document_store::DocumentStore for CountingStore {
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> StoreFuture<'a, Result<String, xc_kernel::document_store::DocumentStoreError>> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.inner.put(doc_ref, bytes)
    }

    fn get<'a>(
        &'a self,
        doc_ref: &'a str,
    ) -> StoreFuture<'a, Result<Vec<u8>, xc_kernel::document_store::DocumentStoreError>> {
        self.inner.get(doc_ref)
    }

    fn delete<'a>(
        &'a self,
        doc_ref: &'a str,
    ) -> StoreFuture<'a, Result<(), xc_kernel::document_store::DocumentStoreError>> {
        self.inner.delete(doc_ref)
    }
}

/// Calls the route with exactly the origin headers given (none added), a counting store, and a
/// valid branch reviewer, customer, idempotency key and multipart body otherwise.
async fn call_with_origin_headers(
    env: &TestEnv,
    origin_headers: &[(&str, &str)],
) -> (HttpAnswer, Arc<CountingStore>) {
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let user_id = env.user(ORG_1).await;
    let spec = format!("{user_id}|VERIFICATION_REVIEWER|{B01}|{ORG_1}");
    let boundary = "bound123";
    let payload = build_multipart(
        boundary,
        Some(&customer_id),
        Some("PASSPORT"),
        Some(("file", "passport.png", &png())),
    );

    let state = AppState::new(Arc::new(common::settings(&[])), env.pool.clone());
    let store = Arc::new(CountingStore {
        inner: state.document_store.clone(),
        puts: AtomicU64::new(0),
    });
    let state = state.with_document_store(store.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::post()
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, spec.as_str()))
        .insert_header(("idempotency-key", unique("origin-key").as_str()))
        .insert_header((
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        ))
        .set_payload(payload);
    for (name, value) in origin_headers {
        req = req.insert_header((*name, *value));
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (HttpAnswer { status, text, json }, store)
}

/// 400 with code `ORIGIN_REJECTED`, in either shape the API uses: this file's
/// `{"error":{"code":..}}` or bookings' `{"error":".."}`.
fn assert_origin_rejected(answer: &HttpAnswer) {
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "a cross-site write must be refused 400, got {} {}",
        answer.status,
        answer.text
    );
    let code = answer.json["error"]["code"]
        .as_str()
        .or_else(|| answer.json["error"].as_str())
        .unwrap_or("");
    assert_eq!(code, "ORIGIN_REJECTED", "body: {}", answer.text);
}

async fn assert_nothing_written(env: &TestEnv, store: &CountingStore) {
    assert_eq!(
        env.count("SELECT count(*) FROM verification_cases").await,
        0,
        "no case may be opened"
    );
    assert_eq!(
        env.count("SELECT count(*) FROM verification_documents")
            .await,
        0,
        "no document row may be written"
    );
    assert_eq!(
        env.count("SELECT count(*) FROM audit_events").await,
        0,
        "no audit row may be written"
    );
    assert_eq!(
        store.puts.load(Ordering::SeqCst),
        0,
        "no document may be stored"
    );
}

#[actix_web::test]
async fn request_without_marker_header_from_foreign_origin_is_refused_and_writes_nothing() {
    let env = TestEnv::start().await;
    let (answer, store) = call_with_origin_headers(
        &env,
        &[("origin", FOREIGN_ORIGIN), ("x-forwarded-proto", "https")],
    )
    .await;
    assert_origin_rejected(&answer);
    assert_nothing_written(&env, &store).await;
    env.finish().await;
}

#[actix_web::test]
async fn request_with_marker_header_from_foreign_origin_is_refused_and_writes_nothing() {
    let env = TestEnv::start().await;
    let (answer, store) = call_with_origin_headers(
        &env,
        &[
            ("origin", FOREIGN_ORIGIN),
            ("x-xchange-request", "1"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert_origin_rejected(&answer);
    assert_nothing_written(&env, &store).await;
    env.finish().await;
}

#[actix_web::test]
async fn request_with_wrong_marker_value_from_own_origin_is_refused_and_writes_nothing() {
    let env = TestEnv::start().await;
    let (answer, store) = call_with_origin_headers(
        &env,
        &[
            ("origin", ORIGIN),
            ("x-xchange-request", "true"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert_origin_rejected(&answer);
    assert_nothing_written(&env, &store).await;
    env.finish().await;
}

#[actix_web::test]
async fn same_origin_request_with_marker_header_still_opens_a_case() {
    let env = TestEnv::start().await;
    let (answer, store) = call_with_origin_headers(
        &env,
        &[
            ("origin", ORIGIN),
            ("x-xchange-request", "1"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "body: {}", answer.text);
    let parsed: BranchCaseOpenAnswer =
        serde_json::from_value(answer.json.clone()).expect("parse BranchCaseOpenAnswer");
    assert_eq!(parsed.case.status, "SUBMITTED");
    assert_eq!(
        env.count("SELECT count(*) FROM verification_cases").await,
        1
    );
    assert_eq!(
        env.count("SELECT count(*) FROM verification_documents")
            .await,
        1
    );
    assert_eq!(env.count("SELECT count(*) FROM audit_events").await, 1);
    assert_eq!(store.puts.load(Ordering::SeqCst), 1);
    env.finish().await;
}
