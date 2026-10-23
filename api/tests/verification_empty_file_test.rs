//! #4925 (server half): a 0-byte verification upload gets its own refusal, not "file over 15 MB", and the branch
//! intake's policy refusals carry a sentence, not the code.
//!
//! Written before the implementation by the ticket author's lane (claude/b873aa20, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Behaviour rows pinned (ticket §3; the screen rows VF3–VF6 are #4940's):
//! - VF1 API, customer upload (`POST /api/app/verification/cases/{id}/documents`, through `CaseService::add_document`)
//!   of a 0-byte file ⇒ 400 `validationFailed` with a sentence that is not the 15 MB one; nothing stored.
//! - VF2 API, branch intake (`POST /api/operations/verification/branch-cases`), the policy refuses the document type
//!   ⇒ 400 `documentTypeNotAccepted` whose message is a sentence, not the code.
//!
//! Harnesses copied from `app_verification_upload_origin_test.rs` and `verification_branch_intake_test.rs`.
//! Needs `TEST_DATABASE_URL`; nothing skips.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
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
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{PiiCipher, PiiKeys};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::document_store::FsDocumentStore;
use xchange::presentation::http::branch_verification_intake::ROUTE as INTAKE;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const USER_CUST_1: &str = "55555555-5555-5555-5555-555555555555";
const B01: &str = "B01";
const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
const BOUNDARY: &str = "---------------------------vfemptyboundary";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn png() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(b"VF2_PASSPORT_PNG_PAYLOAD");
    b
}

fn multipart(fields: &[(&str, &str)], file: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, val) in fields {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{val}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"doc.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

// ------------------------------------------------------------ callers

/// `x-test-as: <user id>|<ROLE>|<branch or empty>|<customer id or empty>`, of `ORG_1`.
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
            let parts: Vec<&str> = spec.split('|').collect();
            let user = (parts.len() == 4)
                .then(|| Role::parse(parts[1]))
                .flatten()
                .map(|role| {
                    let branch = (!parts[2].is_empty()).then(|| BranchId::new(parts[2]));
                    let customer = (!parts[3].is_empty()).then_some(parts[3]);
                    let from = Utc::now() - ChronoDuration::days(30);
                    let mut actor = User::new(
                        parts[0],
                        ORG_1,
                        vec![ScopedRole::new(role, branch.clone(), from, None)],
                        customer,
                    );
                    actor.branch = branch;
                    actor
                });
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ database

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

struct Room {
    pool: PgPool,
    schema: String,
    store_dir: std::path::PathBuf,
}

impl Room {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at a test database ending in `_test`")
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("vfempty");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        let store_dir =
            std::env::temp_dir().join(format!("xc-docstore-vfempty-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&store_dir).expect("create temp docstore dir");
        let room = Self {
            pool,
            schema,
            store_dir,
        };
        room.seed().await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// ORG_1 with a policy accepting PASSPORT and NATIONAL_ID, branch B01, and customer CX_1 linked to USER_CUST_1.
    async fn seed(&self) {
        self.exec(&format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ('{ORG_1}'::uuid, 'ORG1', 'First Org', 'Asia/Bangkok', now()) ON CONFLICT DO NOTHING"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO verification_policies \
             (organization_id, version, accepted_document_types, documents_required, branch_review_allowed, published_by, published_at, created_at) \
             VALUES ('{ORG_1}'::uuid, 1, ARRAY['PASSPORT', 'NATIONAL_ID'], 1, true, NULL, now(), now()) \
             ON CONFLICT DO NOTHING"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{B01}', '{B01}', '{ORG_1}'::uuid) \
             ON CONFLICT DO NOTHING"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by, created_at) \
             VALUES ('{USER_CUST_1}'::uuid, '{ORG_1}'::uuid, 'Customer Linked User', 'ACTIVE', 'seed', now()) \
             ON CONFLICT (id) DO NOTHING"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('CX_1', '{ORG_1}'::uuid, 'CX_1', 'Somchai Prasert', 'LIVE', now(), '{USER_CUST_1}'::uuid) \
             ON CONFLICT DO NOTHING"
        ))
        .await;
    }

    async fn reviewer(&self) -> String {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{id}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'test')",
            unique("reviewer")
        ))
        .await;
        format!("{id}|VERIFICATION_REVIEWER|{B01}|")
    }

    async fn draft_case(&self) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO verification_cases \
             (id, organization_id, customer_id, provider, status, policy_version, version, created_at, updated_at) \
             VALUES ('{id}', '{ORG_1}'::uuid, 'CX_1', 'STAFF', 'DRAFT', 1, 1, now(), now())"
        ))
        .await;
        id
    }

    async fn send(
        &self,
        who: &str,
        uri: &str,
        host: &str,
        origin: &str,
        payload: Vec<u8>,
    ) -> Answer {
        let mut state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        let keys = Arc::new(
            PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("valid test keys"),
        );
        state.document_store = Arc::new(FsDocumentStore::new(
            self.store_dir.clone(),
            PiiCipher::new(keys),
        ));
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let session = format!(
            "__Host-xc_session=session-{}",
            who.split('|').next().unwrap_or("")
        );
        let mut req = atest::TestRequest::default()
            .method(Method::POST)
            .uri(uri)
            .insert_header((header::HOST, host))
            .insert_header(("origin", origin))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((AS, who))
            .insert_header((header::COOKIE, session))
            .insert_header(("idempotency-key", unique("key")))
            .insert_header((
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={BOUNDARY}"),
            ))
            .set_payload(payload);
        if origin.starts_with("https://") {
            req = req.insert_header(("x-forwarded-proto", "https"));
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, text, body }
    }

    async fn finish(self) {
        let Room {
            pool,
            schema,
            store_dir,
        } = self;
        let _ = std::fs::remove_dir_all(&store_dir);
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

#[actix_web::test]
async fn vf1_a_zero_byte_upload_is_refused_as_empty_not_as_over_15_mb() {
    let room = Room::start().await;
    let case_id = room.draft_case().await;
    let a = room
        .send(
            &format!("{USER_CUST_1}|CUSTOMER||CX_1"),
            &format!("/api/app/verification/cases/{case_id}/documents"),
            "xc.test",
            "http://xc.test",
            multipart(&[("documentType", "PASSPORT")], &[]),
        )
        .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "VF1: {}", a.text);
    assert_eq!(
        a.body["error"]["code"].as_str(),
        Some("validationFailed"),
        "VF1: code; body {}",
        a.text
    );
    let message = a.body["error"]["message"].as_str().unwrap_or("");
    assert!(
        !message.is_empty() && message != TOO_LARGE,
        "VF1: the empty file has its own sentence, not the 15 MB one; body {}",
        a.text
    );
    assert_eq!(
        room.count("SELECT count(*) FROM verification_documents")
            .await,
        0,
        "VF1: nothing stored"
    );
    room.finish().await;
}

#[actix_web::test]
async fn vf2_the_intake_policy_refusing_the_document_type_answers_a_sentence_not_the_code() {
    let room = Room::start().await;
    let reviewer = room.reviewer().await;
    let a = room
        .send(
            &reviewer,
            INTAKE,
            "desk.example",
            "https://desk.example",
            multipart(&[("customerId", "CX_1"), ("documentType", "PHONE")], &png()),
        )
        .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "VF2: {}", a.text);
    assert_eq!(
        a.body["error"]["code"].as_str(),
        Some("documentTypeNotAccepted"),
        "VF2: code; body {}",
        a.text
    );
    let message = a.body["error"]["message"].as_str().unwrap_or("");
    assert!(
        !message.is_empty() && message != "documentTypeNotAccepted",
        "VF2: the message is a sentence, not the code; body {}",
        a.text
    );
    assert_eq!(
        room.count("SELECT count(*) FROM verification_cases").await,
        0,
        "VF2: no case opened"
    );
    room.finish().await;
}
