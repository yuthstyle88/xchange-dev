//! Up-front tests for #2705: branch intake that reuses a customer's open online case must file it under the
//! intake branch, so the same branch reviewer can review it (design `docs/design/blueprint/verification.md` V17).
//!
//! Written before the fix by the test-writer claude/dell-claude-1, on the BE lane controller claude/1656900d's
//! order. Do not edit, delete or weaken these tests; add your own in another file.
//!
//! **The bug** (main `d862a5e9`): `BranchIntake` (`crates/xc-verification/src/application/branch_intake.rs`) reuses
//! an open DRAFT / NEEDS_UPDATE case and calls `update_case_submitted`
//! (`crates/xc-verification/src/infrastructure/pg_case_repository.rs`), which never writes `branch_id`. A case
//! opened online (`branch_id` NULL) is submitted with no branch, and `review_service.rs` then refuses the branch
//! reviewer that took it (`outOfScope`).
//!
//! **Rulings:** V3 (an open DRAFT case already filed at another branch) answers **400 `caseLocked`** and leaves the
//! case untouched: the BE controller on #2705 (issuecomment-6224799878, corrected to 400 in the latest comment).
//!
//! | Row | Test |
//! |---|---|
//! | V1 | `v1_open_online_draft_is_filed_under_the_intake_branch_and_reviewable_there` |
//! | V2 | `v2_open_online_needs_update_is_filed_under_the_intake_branch_and_reviewable_there` |
//! | V3 | `v3_open_draft_filed_at_another_branch_is_case_locked_and_untouched` |
//! | V4 | `v4_no_open_case_opens_a_new_case_under_the_intake_branch` (kept) |
//! | V5 | `v5_open_submitted_or_under_review_case_is_case_locked` (kept) |
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`. Each test works in a schema of
//! its own.

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
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
use xchange::presentation::pages::{Callers, SessionReader};

mod common;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "B01";
const B02: &str = "B02";
const HOST: &str = "desk.example";
const ORIGIN: &str = "https://desk.example";
const AS: &str = "x-test-signed-in-as";
const MARKER: &[u8] = b"VF12-HTTP-SECRET-BYTES-3b8c";
const MUST_FINISH: Duration = Duration::from_secs(60);

static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn png() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(MARKER);
    b.extend_from_slice(&unique("png").into_bytes());
    b
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
        let schema = unique("vf2705");
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

// ── #2705 helpers ────────────────────────────────────────────────────────────

impl TestEnv {
    /// The stored state of a case: status, branch_id, version, document count.
    async fn case_row(&self, id: Uuid) -> (String, Option<String>, i32, i64) {
        let (status, branch, version): (String, Option<String>, i32) = sqlx::query_as(
            "SELECT status, branch_id, version FROM verification_cases WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("fetch the case");
        let docs = self
            .count(&format!(
                "SELECT count(*) FROM verification_documents WHERE case_id = '{id}'"
            ))
            .await;
        (status, branch, version, docs)
    }

    /// One JSON POST to a v3 command, as `who_spec`.
    async fn post_json(&self, who_spec: &str, uri: &str, body: Value) -> HttpAnswer {
        let state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::post()
            .uri(uri)
            .insert_header((header::HOST, HOST))
            .insert_header(("origin", ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("x-forwarded-proto", "https"))
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", unique("k")))
            .insert_header((AS, who_spec))
            .set_payload(body.to_string())
            .to_request();
        let resp = atest::call_service(&app, req).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        HttpAnswer { status, text, json }
    }
}

fn error_code(a: &HttpAnswer) -> &str {
    a.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| a.json["error"].as_str().unwrap_or(""))
}

/// A reviewer of `branch` in ORG_1, and the spec that signs them in.
async fn reviewer(env: &TestEnv, branch: &str) -> String {
    let id = env.user(ORG_1).await;
    format!("{id}|VERIFICATION_REVIEWER|{branch}|{ORG_1}")
}

async fn intake(env: &TestEnv, spec: &str, customer_id: &str) -> HttpAnswer {
    let boundary = "bound2705";
    let payload = build_multipart(
        boundary,
        Some(customer_id),
        Some("PASSPORT"),
        Some(("file", "passport.png", &png())),
    );
    env.call_intake(Some(spec), Some(&unique("idem")), boundary, payload)
        .await
}

/// V1/V2: intake at B01 reuses the case, files it under B01 in the answer and the database, and the same
/// reviewer can start its review.
async fn assert_reused_under_b01_and_reviewable(status: &str, what: &str) {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let case_id = env.force_open_case(&customer_id, ORG_1, status).await;
    let spec = reviewer(&env, B01).await;

    let answer = intake(&env, &spec, &customer_id).await;

    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{what} intake: {}",
        answer.text
    );
    let parsed: BranchCaseOpenAnswer =
        serde_json::from_value(answer.json.clone()).expect("parse BranchCaseOpenAnswer");
    assert_eq!(parsed.case.id, case_id, "{what}: the open case is reused");
    assert_eq!(parsed.case.status, "SUBMITTED", "{what}");
    assert_eq!(
        parsed.case.branch_id.as_deref(),
        Some(B01),
        "{what}: the answer files the reused case under the intake branch (V17)"
    );
    let (_, db_branch, version, _) = env.case_row(case_id).await;
    assert_eq!(
        db_branch.as_deref(),
        Some(B01),
        "{what}: verification_cases.branch_id is the intake branch"
    );

    let review = env
        .post_json(
            &spec,
            &format!("/api/operations/verification/cases/{case_id}/start-review"),
            json!({ "expectedVersion": version }),
        )
        .await;
    assert_eq!(
        review.status,
        StatusCode::OK,
        "{what}: the intake reviewer can start the review of the case it took; got {}",
        review.text
    );
    env.finish().await;
}

// ── V1–V5 ───────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn v1_open_online_draft_is_filed_under_the_intake_branch_and_reviewable_there() {
    assert_reused_under_b01_and_reviewable("DRAFT", "V1").await;
}

#[actix_web::test]
async fn v2_open_online_needs_update_is_filed_under_the_intake_branch_and_reviewable_there() {
    assert_reused_under_b01_and_reviewable("NEEDS_UPDATE", "V2").await;
}

/// V3 (ruled): the open DRAFT case is already filed at B02. Intake at B01 answers 400 `caseLocked`, and the case
/// keeps its branch, status, version and documents.
#[actix_web::test]
async fn v3_open_draft_filed_at_another_branch_is_case_locked_and_untouched() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    env.branch(ORG_1, B02).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let case_id = env.force_open_case(&customer_id, ORG_1, "DRAFT").await;
    env.exec(&format!(
        "UPDATE verification_cases SET branch_id = '{B02}' WHERE id = '{case_id}'"
    ))
    .await;
    let before = env.case_row(case_id).await;
    let spec = reviewer(&env, B01).await;

    let answer = intake(&env, &spec, &customer_id).await;

    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "V3: {}",
        answer.text
    );
    assert_eq!(error_code(&answer), "caseLocked", "V3: {}", answer.text);
    assert_eq!(
        env.case_row(case_id).await,
        before,
        "V3: the case filed at B02 is left untouched"
    );
    assert_eq!(
        env.count("SELECT count(*) FROM verification_cases").await,
        1,
        "V3: no other case is opened"
    );
    env.finish().await;
}

/// V4 (kept): no open case. Intake opens a new SUBMITTED case under B01.
#[actix_web::test]
async fn v4_no_open_case_opens_a_new_case_under_the_intake_branch() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let spec = reviewer(&env, B01).await;

    let answer = intake(&env, &spec, &customer_id).await;

    assert_eq!(answer.status, StatusCode::OK, "V4: {}", answer.text);
    let parsed: BranchCaseOpenAnswer =
        serde_json::from_value(answer.json.clone()).expect("parse BranchCaseOpenAnswer");
    assert_eq!(parsed.case.status, "SUBMITTED");
    assert_eq!(parsed.case.branch_id.as_deref(), Some(B01));
    env.finish().await;
}

/// V5 (kept): an open SUBMITTED or UNDER_REVIEW case answers 400 `caseLocked`, and the case does not change.
#[actix_web::test]
async fn v5_open_submitted_or_under_review_case_is_case_locked() {
    for status in ["SUBMITTED", "UNDER_REVIEW"] {
        let env = TestEnv::start().await;
        env.branch(ORG_1, B01).await;
        let (customer_id, _) = env.customer(ORG_1).await;
        let case_id = env.force_open_case(&customer_id, ORG_1, status).await;
        let before = env.case_row(case_id).await;
        let spec = reviewer(&env, B01).await;

        let answer = intake(&env, &spec, &customer_id).await;

        assert_eq!(
            answer.status,
            StatusCode::BAD_REQUEST,
            "V5 {status}: {}",
            answer.text
        );
        assert_eq!(
            error_code(&answer),
            "caseLocked",
            "V5 {status}: {}",
            answer.text
        );
        assert_eq!(
            env.case_row(case_id).await,
            before,
            "V5 {status}: unchanged"
        );
        env.finish().await;
    }
}
