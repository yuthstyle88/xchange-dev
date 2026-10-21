//! VF-8 #1399 — customer verification HTTP API (C10 commands, my_verification view, app upload route).
//! Design `docs/design/blueprint/verification.md` §4.4, §7, §8, §9.1 row VF-8; ticket #1399 §4, §8, §9.
//!
//! Written before the implementation by the test writer gemini/studio-14, revised by gemini/studio-11 (controller claude/553782). Do not edit, delete or weaken.
//!
//! Behavior & Edge Cases locked by these tests:
//! 1. `every_c10_route_refuses_without_customer_self`:
//!    All 5 commands + 1 view refuse when caller lacks `CustomerSelf` (TELLER, ADMIN, EXECUTIVE, VERIFICATION_REVIEWER, unsigned).
//! 2. `signed_in_user_with_no_customer_id_is_refused`:
//!    A user signed in as CUSTOMER but having no `customer_id` is refused on C10 routes.
//! 3. `c10_routes_refuse_for_another_customers_case_and_document`:
//!    Customer B attempting to upload to Customer A's case, remove Customer A's document, submit Customer A's case, or link Customer A's document is refused with `outOfScope` (HTTP 403).
//! 4. `verification_case_open_returns_existing_open_case_or_creates_new_draft`:
//!    `verificationCaseOpen` creates a DRAFT case if none exists, or returns the existing open case if one is already open.
//! 5. `verification_case_open_after_rejected_or_expired_opens_new_case`:
//!    When previous case is REJECTED or EXPIRED, `verificationCaseOpen` opens a new DRAFT case.
//! 6. `verification_case_open_refuses_when_no_verification_policy`:
//!    Calling `verificationCaseOpen` for an organization with no policy returns `noVerificationPolicy`.
//! 7. `document_upload_stores_bytes_in_document_store_not_in_database`:
//!    Document upload stores bytes in `DocumentStore`, leaves `files` table row count unchanged, writes no `bytea` to the database, and records metadata in `verification_documents`.
//! 8. `upload_refuses_when_case_locked_in_submitted_or_reviewer_states`:
//!    Upload returns `caseLocked` when case is in SUBMITTED, UNDER_REVIEW, VERIFIED, REJECTED, EXPIRED, or REVOKED.
//! 9. `upload_and_submit_allowed_when_case_in_needs_update`:
//!    When case is in NEEDS_UPDATE, upload and submit are allowed again.
//! 10. `upload_refuses_document_type_not_accepted_by_policy`:
//!     Uploading a document type not accepted by the policy returns `documentTypeNotAccepted`.
//! 11. `upload_over_15mb_is_refused_with_file_service_too_large_sentence`:
//!     Upload over 15 MB is refused with `file_service.rs` sentence (`TOO_LARGE`, HTTP 400).
//! 12. `document_remove_succeeds_in_draft_and_refuses_when_case_locked`:
//!     `verificationDocumentRemove` removes an attached document in DRAFT or NEEDS_UPDATE, but returns `caseLocked` when case is SUBMITTED.
//! 13. `case_submit_moves_draft_to_submitted_and_refuses_when_documents_missing`:
//!     `verificationCaseSubmit` moves DRAFT to SUBMITTED when required documents are attached; returns `documentsMissing` if required documents are missing.
//! 14. `my_document_link_issues_download_link_for_owner_only`:
//!     `verificationMyDocumentLink` returns `{url, expiresAt}` for the case owner, writes an audit row, and refuses another customer with `outOfScope`.
//! 15. `my_verification_view_carries_no_image_bytes_and_shows_metadata_and_readiness`:
//!     `my_verification` view returns newest/open case status, reason code, expiresAt, documents metadata, and readiness without raw image bytes or base64 data.
//! 16. `idempotency_key_replay_returns_cached_response_and_conflict_on_different_body`:
//!     Idempotency-Key replay returns the first answer; same key with a different body returns `idempotencyConflict` (HTTP 409).
//! 17. `no_customer_command_can_transition_to_reviewer_state`:
//!     No C10 command reaches or transitions a case to reviewer states (UNDER_REVIEW, VERIFIED, REJECTED, REVOKED).

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
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const ORG_2: &str = "00000000-0000-0000-0000-000000000002";
const ORG_NO_POLICY: &str = "00000000-0000-0000-0000-000000000099";
const BRANCH_MAIN: &str = "MAIN";

const USER_REVIEWER: &str = "11111111-1111-1111-1111-111111111111";
const USER_CUST_1: &str = "55555555-5555-5555-5555-555555555555";
const USER_CUST_2: &str = "66666666-6666-6666-6666-666666666666";
const USER_ADMIN: &str = "77777777-7777-7777-7777-777777777777";
const USER_EXEC: &str = "88888888-8888-8888-8888-888888888888";
const USER_TELLER: &str = "99999999-9999-9999-9999-999999999999";

// Starts with the PNG signature: VF-3's `CaseService::add_document` judges the file type by
// content (design verification.md §5), so a seed without magic bytes is refused before any
// ownership or state check runs. The rest is a recognisable marker for the leak checks.
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

fn base64_encode(bytes: &[u8], url_safe: bool, pad: bool) -> String {
    let table = if url_safe {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(table[((n >> 18) & 63) as usize] as char);
        out.push(table[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(table[((n >> 6) & 63) as usize] as char);
        } else if pad {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(table[(n & 63) as usize] as char);
        } else if pad {
            out.push('=');
        }
    }
    out
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

    fn customer_no_id(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Customer,
            branch: None,
            customer_id: None,
            org_id: ORG_1.to_owned(),
            session: format!("session-{user_id}"),
        }
    }

    fn customer_no_policy_org(user_id: &str, customer_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Customer,
            branch: None,
            customer_id: Some(customer_id.to_owned()),
            org_id: ORG_NO_POLICY.to_owned(),
            session: format!("session-{user_id}"),
        }
    }

    fn org_reviewer(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::VerificationReviewer,
            branch: None,
            customer_id: None,
            org_id: ORG_1.to_owned(),
            session: format!("session-{user_id}"),
        }
    }

    fn teller(user_id: &str, branch: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Teller,
            branch: Some(branch.to_owned()),
            customer_id: None,
            org_id: ORG_1.to_owned(),
            session: format!("session-{user_id}"),
        }
    }

    fn admin(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Admin,
            branch: None,
            customer_id: None,
            org_id: ORG_1.to_owned(),
            session: format!("session-{user_id}"),
        }
    }

    fn executive(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Executive,
            branch: None,
            customer_id: None,
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

        let schema = unique("c10");
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
        // Seed ORG_1 and policy
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

        // Seed ORG_2 and policy
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, 'ORG2', 'Second Org', 'Asia/Bangkok', now()) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_2).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed org2");

        sqlx::query(
            "INSERT INTO verification_policies \
             (organization_id, version, accepted_document_types, documents_required, branch_review_allowed, published_by, published_at, created_at) \
             VALUES ($1, 1, ARRAY['PASSPORT', 'NATIONAL_ID'], 1, true, NULL, now(), now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_2).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed org2 policy");

        // Seed ORG_NO_POLICY without any verification_policies row
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, 'ORG99', 'No Policy Org', 'Asia/Bangkok', now()) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_NO_POLICY).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed org no policy");

        // Seed branches
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

        // Seed users
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ($1, $7, 'Org Reviewer', 'ACTIVE', 'seed'), \
             ($2, $7, 'Customer User 1', 'ACTIVE', 'seed'), \
             ($3, $7, 'Customer User 2', 'ACTIVE', 'seed'), \
             ($4, $7, 'Admin User', 'ACTIVE', 'seed'), \
             ($5, $7, 'Executive User', 'ACTIVE', 'seed'), \
             ($6, $7, 'Teller User', 'ACTIVE', 'seed') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::from_str(USER_REVIEWER).unwrap())
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .bind(Uuid::from_str(USER_CUST_2).unwrap())
        .bind(Uuid::from_str(USER_ADMIN).unwrap())
        .bind(Uuid::from_str(USER_EXEC).unwrap())
        .bind(Uuid::from_str(USER_TELLER).unwrap())
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed users");

        // #1943 / #1399 (authorized by claude/7ca80c): seed live `user_sessions` rows for the
        // test cookies (`hash` = sha256 hex of the cookie), so `caller_session_hash` binds the
        // link to the caller's own live session with no fallback.
        for (cookie, user) in [
            ("session-user-cust-1", USER_CUST_1),
            ("session-user-cust-2", USER_CUST_2),
            ("session-user-cust-b", USER_CUST_2),
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

        // Seed customers
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, user_id, created_at) VALUES \
             ('CX_1', $1, 'CX_1', 'Somchai Prasert', 'LIVE', $3, now()), \
             ('CX_2', $1, 'CX_2', 'Somsri Sukjai', 'LIVE', $4, now()), \
             ('CX_NOPOLICY', $2, 'CX_NOPOLICY', 'No Policy Customer', 'LIVE', NULL, now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(Uuid::from_str(ORG_NO_POLICY).unwrap())
        .bind(Uuid::from_str(USER_CUST_1).unwrap())
        .bind(Uuid::from_str(USER_CUST_2).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed customers");

        // Seed customer identity for CX_1
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

    #[allow(clippy::too_many_arguments)]
    async fn insert_case(
        &self,
        case_id: Uuid,
        org_id: &str,
        customer_id: &str,
        status: &str,
        branch_id: Option<&str>,
        submitted_at: Option<DateTime<Utc>>,
        decided_at: Option<DateTime<Utc>>,
        expires_at: Option<DateTime<Utc>>,
        reviewed_by: Option<Uuid>,
        decision_reason_code: Option<&str>,
        version: i32,
    ) {
        sqlx::query(
            "INSERT INTO verification_cases \
             (id, organization_id, customer_id, provider, external_case_ref, status, policy_version, branch_id, \
              submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'STAFF', NULL, $4, 1, $5, $6, $7, $8, $9, $10, $11, now(), now())",
        )
        .bind(case_id)
        .bind(Uuid::from_str(org_id).unwrap())
        .bind(customer_id)
        .bind(status)
        .bind(branch_id)
        .bind(submitted_at)
        .bind(decided_at)
        .bind(expires_at)
        .bind(reviewed_by.map(|_| Uuid::from_str(USER_REVIEWER).unwrap()))
        .bind(decision_reason_code)
        .bind(version)
        .execute(&self.pool)
        .await
        .expect("insert verification case");
    }

    /// V14 (`verification_one_open_case`, migration 20261205121000 line 52) allows one open case per customer.
    /// A test that seeds a second case for the same customer first closes the earlier open one as EXPIRED,
    /// so the insert does not fail with 23505. EXPIRED needs no reviewer, decision time or reason code.
    async fn close_open_cases(&self, customer_id: &str) {
        sqlx::query(
            "UPDATE verification_cases SET status = 'EXPIRED', updated_at = now() \
             WHERE customer_id = $1 AND status IN ('DRAFT', 'SUBMITTED', 'UNDER_REVIEW', 'NEEDS_UPDATE')",
        )
        .bind(customer_id)
        .execute(&self.pool)
        .await
        .expect("close open verification cases");
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
        _uploaded_by: Uuid,
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
    cookie: Option<&str>,
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
        req = req.insert_header((
            header::COOKIE,
            cookie.unwrap_or(session_cookie.as_str()).to_string(),
        ));
    } else if let Some(c) = cookie {
        req = req.insert_header((header::COOKIE, c.to_string()));
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
// Test 1: Every C10 route refuses without CustomerSelf (signed out or staff role)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn every_c10_route_refuses_without_customer_self() {
    let room = Room::start().await;
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-1"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
        Uuid::new_v4(),
    )
    .await;

    let unauth_callers = [
        (
            "teller",
            Who::teller("33333333-3333-3333-3333-333333333333", BRANCH_MAIN),
        ),
        ("admin", Who::admin("11111111-1111-1111-1111-111111111111")),
        (
            "executive",
            Who::executive("22222222-2222-2222-2222-222222222222"),
        ),
        (
            "reviewer",
            Who::org_reviewer("44444444-4444-4444-4444-444444444444"),
        ),
    ];

    let boundary = "---------------------------c10testboundary";
    let upload_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", SECRET_DOC_BYTES)),
    );

    // 1. Unauthenticated (signed-out) caller -> 401 with code signIn
    // 1.1 verificationCaseOpen
    let unauth_open = call(
        &room,
        None,
        Method::POST,
        "/api/app/verification/cases",
        Some((json!({}), "IDEM-REFUSE-OPEN-UNAUTH")),
        None,
    )
    .await;
    assert_eq!(
        unauth_open.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated verificationCaseOpen must answer 401: got {}",
        unauth_open.status
    );
    assert_eq!(
        unauth_open.code(),
        "signIn",
        "unauthenticated verificationCaseOpen code was '{}'",
        unauth_open.code()
    );

    // 1.2 document upload
    let unauth_upload = call_multipart(
        &room,
        None,
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        upload_payload.clone(),
        Some("IDEM-REFUSE-UPLOAD-UNAUTH"),
    )
    .await;
    assert_eq!(
        unauth_upload.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated upload must answer 401: got {}",
        unauth_upload.status
    );
    assert_eq!(
        unauth_upload.code(),
        "signIn",
        "unauthenticated upload code was '{}'",
        unauth_upload.code()
    );

    // 1.3 verificationDocumentRemove
    let unauth_remove = call(
        &room,
        None,
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REFUSE-REMOVE-UNAUTH")),
        None,
    )
    .await;
    assert_eq!(
        unauth_remove.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated verificationDocumentRemove must answer 401: got {}",
        unauth_remove.status
    );
    assert_eq!(
        unauth_remove.code(),
        "signIn",
        "unauthenticated verificationDocumentRemove code was '{}'",
        unauth_remove.code()
    );

    // 1.4 verificationCaseSubmit
    let unauth_submit = call(
        &room,
        None,
        Method::POST,
        &format!("/api/app/verification/cases/{case_id}/submit"),
        Some((json!({}), "IDEM-REFUSE-SUBMIT-UNAUTH")),
        None,
    )
    .await;
    assert_eq!(
        unauth_submit.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated verificationCaseSubmit must answer 401: got {}",
        unauth_submit.status
    );
    assert_eq!(
        unauth_submit.code(),
        "signIn",
        "unauthenticated verificationCaseSubmit code was '{}'",
        unauth_submit.code()
    );

    // 1.5 verificationMyDocumentLink
    let unauth_link = call(
        &room,
        None,
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/link"),
        Some((json!({}), "IDEM-REFUSE-LINK-UNAUTH")),
        None,
    )
    .await;
    assert_eq!(
        unauth_link.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated verificationMyDocumentLink must answer 401: got {}",
        unauth_link.status
    );
    assert_eq!(
        unauth_link.code(),
        "signIn",
        "unauthenticated verificationMyDocumentLink code was '{}'",
        unauth_link.code()
    );

    // 1.6 my_verification view
    let unauth_view = call(
        &room,
        None,
        Method::GET,
        "/api/v3/views/my_verification",
        None,
        None,
    )
    .await;
    assert_eq!(
        unauth_view.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated my_verification view must answer 401: got {}",
        unauth_view.status
    );
    assert_eq!(
        unauth_view.code(),
        "signIn",
        "unauthenticated my_verification view code was '{}'",
        unauth_view.code()
    );

    // 2. Staff callers without CustomerSelf -> 400 roleNotAllowedHere or 403 outOfScope
    for (label, who) in &unauth_callers {
        // 2.1 verificationCaseOpen
        let ans_open = call(
            &room,
            Some(who),
            Method::POST,
            "/api/app/verification/cases",
            Some((json!({}), "IDEM-REFUSE-OPEN-STAFF")),
            None,
        )
        .await;
        assert!(
            ans_open.status == StatusCode::FORBIDDEN || ans_open.status == StatusCode::BAD_REQUEST,
            "{label} call to verificationCaseOpen must be refused with 403 or 400: got {}",
            ans_open.status
        );
        let code = ans_open.code();
        assert!(
            code == "outOfScope" || code == "roleNotAllowedHere",
            "{label} call to verificationCaseOpen code was '{code}'"
        );

        // 2.2 document upload
        let ans_upload = call_multipart(
            &room,
            Some(who),
            &format!("/api/app/verification/cases/{case_id}/documents"),
            boundary,
            upload_payload.clone(),
            Some("IDEM-REFUSE-UPLOAD-STAFF"),
        )
        .await;
        assert!(
            ans_upload.status == StatusCode::FORBIDDEN
                || ans_upload.status == StatusCode::BAD_REQUEST,
            "{label} call to upload must be refused with 403 or 400: got {}",
            ans_upload.status
        );
        let code = ans_upload.code();
        assert!(
            code == "outOfScope" || code == "roleNotAllowedHere",
            "{label} call to upload code was '{code}'"
        );

        // 2.3 verificationDocumentRemove
        let ans_remove = call(
            &room,
            Some(who),
            Method::POST,
            &format!("/api/app/verification/documents/{doc_id}/remove"),
            Some((json!({}), "IDEM-REFUSE-REMOVE-STAFF")),
            None,
        )
        .await;
        assert!(
            ans_remove.status == StatusCode::FORBIDDEN
                || ans_remove.status == StatusCode::BAD_REQUEST,
            "{label} call to verificationDocumentRemove must be refused with 403 or 400: got {}",
            ans_remove.status
        );
        let code = ans_remove.code();
        assert!(
            code == "outOfScope" || code == "roleNotAllowedHere",
            "{label} call to verificationDocumentRemove code was '{code}'"
        );

        // 2.4 verificationCaseSubmit
        let ans_submit = call(
            &room,
            Some(who),
            Method::POST,
            &format!("/api/app/verification/cases/{case_id}/submit"),
            Some((json!({}), "IDEM-REFUSE-SUBMIT-STAFF")),
            None,
        )
        .await;
        assert!(
            ans_submit.status == StatusCode::FORBIDDEN
                || ans_submit.status == StatusCode::BAD_REQUEST,
            "{label} call to verificationCaseSubmit must be refused with 403 or 400: got {}",
            ans_submit.status
        );
        let code = ans_submit.code();
        assert!(
            code == "outOfScope" || code == "roleNotAllowedHere",
            "{label} call to verificationCaseSubmit code was '{code}'"
        );

        // 2.5 verificationMyDocumentLink
        let ans_link = call(
            &room,
            Some(who),
            Method::POST,
            &format!("/api/app/verification/documents/{doc_id}/link"),
            Some((json!({}), "IDEM-REFUSE-LINK-STAFF")),
            None,
        )
        .await;
        assert!(
            ans_link.status == StatusCode::FORBIDDEN || ans_link.status == StatusCode::BAD_REQUEST,
            "{label} call to verificationMyDocumentLink must be refused with 403 or 400: got {}",
            ans_link.status
        );
        let code = ans_link.code();
        assert!(
            code == "outOfScope" || code == "roleNotAllowedHere",
            "{label} call to verificationMyDocumentLink code was '{code}'"
        );

        // 2.6 my_verification view
        let ans_view = call(
            &room,
            Some(who),
            Method::GET,
            "/api/v3/views/my_verification",
            None,
            None,
        )
        .await;
        assert!(
            ans_view.status == StatusCode::FORBIDDEN || ans_view.status == StatusCode::BAD_REQUEST,
            "{label} call to my_verification view must be refused with 403 or 400: got {}",
            ans_view.status
        );
        let code = ans_view.code();
        assert!(
            code == "outOfScope" || code == "roleNotAllowedHere",
            "{label} call to my_verification view code was '{code}'"
        );
    }

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 2: Signed-in user with no customer_id is refused
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn signed_in_user_with_no_customer_id_is_refused() {
    let room = Room::start().await;
    let customer_no_id = Who::customer_no_id("no-cust-user-1");

    let ans_open = call(
        &room,
        Some(&customer_no_id),
        Method::POST,
        "/api/app/verification/cases",
        Some((json!({}), "IDEM-NO-CUST-ID")),
        None,
    )
    .await;
    assert_eq!(
        ans_open.status,
        StatusCode::FORBIDDEN,
        "caller with no customer_id must be refused with 403, got {}: {}",
        ans_open.status,
        ans_open.text
    );
    assert_eq!(
        ans_open.code(),
        "outOfScope",
        "caller with no customer_id must have code 'outOfScope', got: {}",
        ans_open.code()
    );

    let ans_view = call(
        &room,
        Some(&customer_no_id),
        Method::GET,
        "/api/v3/views/my_verification",
        None,
        None,
    )
    .await;
    assert_eq!(
        ans_view.status,
        StatusCode::FORBIDDEN,
        "caller with no customer_id on my_verification must be refused with 403, got {}: {}",
        ans_view.status,
        ans_view.text
    );
    assert_eq!(
        ans_view.code(),
        "outOfScope",
        "caller with no customer_id on my_verification must have code 'outOfScope', got: {}",
        ans_view.code()
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 3: C10 routes refuse for another customer's case and document (outOfScope)
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn c10_routes_refuse_for_another_customers_case_and_document() {
    let room = Room::start().await;
    let cust_b = Who::customer("user-cust-b", "CX_2");
    let case_a_id = Uuid::new_v4();
    let doc_a_id = Uuid::new_v4();

    room.insert_case(
        case_a_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_a_id,
        ORG_1,
        case_a_id,
        "PASSPORT",
        Some("ref-doc-a"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
        Uuid::new_v4(),
    )
    .await;

    let boundary = "---------------------------c10crossboundary";
    let upload_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", SECRET_DOC_BYTES)),
    );

    // 1. Upload to CX_1's case by CX_2
    let ans_upload = call_multipart(
        &room,
        Some(&cust_b),
        &format!("/api/app/verification/cases/{case_a_id}/documents"),
        boundary,
        upload_payload,
        Some("IDEM-CROSS-UPLOAD"),
    )
    .await;
    assert_eq!(
        ans_upload.status,
        StatusCode::FORBIDDEN,
        "upload to another customer's case must return 403: {}",
        ans_upload.text
    );
    assert_eq!(ans_upload.code(), "outOfScope");

    // 2. Remove CX_1's document by CX_2
    let ans_remove = call(
        &room,
        Some(&cust_b),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_a_id}/remove"),
        Some((json!({}), "IDEM-CROSS-REMOVE")),
        None,
    )
    .await;
    assert_eq!(
        ans_remove.status,
        StatusCode::FORBIDDEN,
        "remove another customer's document must return 403: {}",
        ans_remove.text
    );
    assert_eq!(ans_remove.code(), "outOfScope");

    // 3. Submit CX_1's case by CX_2
    let ans_submit = call(
        &room,
        Some(&cust_b),
        Method::POST,
        &format!("/api/app/verification/cases/{case_a_id}/submit"),
        Some((json!({}), "IDEM-CROSS-SUBMIT")),
        None,
    )
    .await;
    assert_eq!(
        ans_submit.status,
        StatusCode::FORBIDDEN,
        "submit another customer's case must return 403: {}",
        ans_submit.text
    );
    assert_eq!(ans_submit.code(), "outOfScope");

    // 4. Link CX_1's document by CX_2
    let ans_link = call(
        &room,
        Some(&cust_b),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_a_id}/link"),
        Some((json!({}), "IDEM-CROSS-LINK")),
        None,
    )
    .await;
    assert_eq!(
        ans_link.status,
        StatusCode::FORBIDDEN,
        "link another customer's document must return 403: {}",
        ans_link.text
    );
    assert_eq!(ans_link.code(), "outOfScope");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 4: verificationCaseOpen returns existing open case or creates new DRAFT
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn verification_case_open_returns_existing_open_case_or_creates_new_draft() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");

    // 1. Initial call: no case exists, so a new DRAFT case is opened
    let ans_1 = call(
        &room,
        Some(&cust),
        Method::POST,
        "/api/app/verification/cases",
        Some((json!({}), "IDEM-OPEN-NEW")),
        None,
    )
    .await;
    let data_1 = ans_1.ok("open new case");
    let case_id_1 = data_1["id"].as_str().expect("case id").to_owned();
    assert_eq!(data_1["status"].as_str().expect("status"), "DRAFT");

    // 2. Second call with different idempotency key: existing open case is returned
    let ans_2 = call(
        &room,
        Some(&cust),
        Method::POST,
        "/api/app/verification/cases",
        Some((json!({}), "IDEM-OPEN-EXISTING")),
        None,
    )
    .await;
    let data_2 = ans_2.ok("open existing case");
    let case_id_2 = data_2["id"].as_str().expect("case id").to_owned();
    assert_eq!(
        case_id_1, case_id_2,
        "calling open while a case is open must return the existing case"
    );
    assert_eq!(data_2["status"].as_str().expect("status"), "DRAFT");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 5: verificationCaseOpen after REJECTED or EXPIRED opens a new case
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn verification_case_open_after_rejected_or_expired_opens_new_case() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let old_case_id = Uuid::new_v4();

    // 1. Previous case was REJECTED
    room.insert_case(
        old_case_id,
        ORG_1,
        "CX_1",
        "REJECTED",
        None,
        Some(Utc::now() - ChronoDuration::days(2)),
        Some(Utc::now() - ChronoDuration::days(1)),
        None,
        Some(Uuid::new_v4()),
        Some("DOCUMENT_UNREADABLE"),
        2,
    )
    .await;

    let ans = call(
        &room,
        Some(&cust),
        Method::POST,
        "/api/app/verification/cases",
        Some((json!({}), "IDEM-OPEN-AFTER-REJECT")),
        None,
    )
    .await;
    let data = ans.ok("open case after rejected");
    let new_case_id = data["id"].as_str().expect("new case id").to_owned();
    assert_ne!(
        new_case_id,
        old_case_id.to_string(),
        "after REJECTED, verificationCaseOpen must open a brand new case"
    );
    assert_eq!(data["status"].as_str().expect("status"), "DRAFT");

    // 2. Previous case was EXPIRED (V2)
    let cust_expired = Who::customer("user-cust-2", "CX_2");
    let expired_case_id = Uuid::new_v4();
    room.insert_case(
        expired_case_id,
        ORG_1,
        "CX_2",
        "EXPIRED",
        None,
        Some(Utc::now() - ChronoDuration::days(30)),
        Some(Utc::now() - ChronoDuration::days(29)),
        Some(Utc::now() - ChronoDuration::days(1)),
        Some(Uuid::new_v4()),
        None,
        2,
    )
    .await;

    let ans_expired = call(
        &room,
        Some(&cust_expired),
        Method::POST,
        "/api/app/verification/cases",
        Some((json!({}), "IDEM-OPEN-AFTER-EXPIRED")),
        None,
    )
    .await;
    let data_expired = ans_expired.ok("open case after expired");
    let new_expired_case_id = data_expired["id"].as_str().expect("new case id").to_owned();
    assert_ne!(
        new_expired_case_id,
        expired_case_id.to_string(),
        "after EXPIRED, verificationCaseOpen must open a brand new case"
    );
    assert_eq!(data_expired["status"].as_str().expect("status"), "DRAFT");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 6: verificationCaseOpen refuses when no verification policy exists
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn verification_case_open_refuses_when_no_verification_policy() {
    let room = Room::start().await;
    let cust_no_policy = Who::customer_no_policy_org("user-no-policy", "CX_NOPOLICY");

    let ans = call(
        &room,
        Some(&cust_no_policy),
        Method::POST,
        "/api/app/verification/cases",
        Some((json!({}), "IDEM-NO-POLICY")),
        None,
    )
    .await;
    assert!(
        ans.status == StatusCode::BAD_REQUEST || ans.status == StatusCode::CONFLICT,
        "verificationCaseOpen must refuse when organisation has no policy, got {}: {}",
        ans.status,
        ans.text
    );
    assert_eq!(ans.code(), "noVerificationPolicy");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 7: Document upload stores bytes in DocumentStore, not in database
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn document_upload_stores_bytes_in_document_store_not_in_database() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    // Check files row count before upload
    let files_count_before: i64 = sqlx::query_scalar("SELECT count(*) FROM files")
        .fetch_one(&room.pool)
        .await
        .expect("count files before");

    let boundary = "---------------------------c10storecheckboundary";
    let upload_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", SECRET_DOC_BYTES)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        upload_payload,
        Some("IDEM-UPLOAD-STORE-CHECK"),
    )
    .await;
    let data = ans.ok("upload document");
    let doc_id_str = data["id"].as_str().expect("document id");
    let doc_uuid = Uuid::from_str(doc_id_str).expect("valid doc uuid");

    // 1. Files table count remains identical (row count of files unchanged)
    let files_count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM files")
        .fetch_one(&room.pool)
        .await
        .expect("count files after");
    assert_eq!(
        files_count_before, files_count_after,
        "upload must not write rows to files table"
    );

    // 2. Query verification_documents: metadata is saved, encrypted_object_ref exists
    let (object_ref, checksum, mime, size_bytes): (String, Vec<u8>, String, i32) = sqlx::query_as(
        "SELECT encrypted_object_ref, checksum, mime, size_bytes FROM verification_documents WHERE id = $1",
    )
    .bind(doc_uuid)
    .fetch_one(&room.pool)
    .await
    .expect("fetch verification_document row");

    assert_eq!(mime, "image/png");
    assert_eq!(size_bytes, SECRET_DOC_BYTES.len() as i32);
    let expected_checksum = Sha256::digest(SECRET_DOC_BYTES);
    assert_eq!(&checksum[..], &expected_checksum[..]);

    // 3. Bytes are in DocumentStore, not in DB
    let stored_bytes = room
        .store
        .get(&object_ref)
        .await
        .expect("get bytes from document store");
    assert_eq!(stored_bytes, SECRET_DOC_BYTES);

    // 4. Assert SECRET_DOC_BYTES is in no bytea column of the test schema
    let bytea_columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name, column_name \
         FROM information_schema.columns \
         WHERE table_schema = $1 AND data_type = 'bytea'",
    )
    .bind(&room.schema)
    .fetch_all(&room.pool)
    .await
    .expect("scan bytea columns in test schema");

    assert!(
        !bytea_columns.is_empty(),
        "test schema must have bytea columns (e.g. checksum)"
    );

    for (table, col) in bytea_columns {
        let query_str = format!(
            "SELECT count(*) FROM \"{}\".\"{}\" WHERE \"{}\" = $1",
            room.schema, table, col
        );
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(query_str))
            .bind(SECRET_DOC_BYTES)
            .fetch_one(&room.pool)
            .await
            .unwrap_or_else(|e| panic!("failed to scan bytea column {table}.{col}: {e}"));
        assert_eq!(
            count, 0,
            "raw SECRET_DOC_BYTES found in table {table} column {col} (expected no bytea written to database)"
        );
    }

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 8: Upload refuses when case locked in SUBMITTED or reviewer states
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn upload_refuses_when_case_locked_in_submitted_or_reviewer_states() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");

    let boundary = "---------------------------c10lockboundary";
    let upload_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "passport.png", "image/png", SECRET_DOC_BYTES)),
    );

    let locked_states = [
        "SUBMITTED",
        "UNDER_REVIEW",
        "VERIFIED",
        "REJECTED",
        "EXPIRED",
        "REVOKED",
    ];

    for status in locked_states {
        let case_id = Uuid::new_v4();
        room.close_open_cases("CX_1").await;
        // The table's CHECKs: VERIFIED, REJECTED and REVOKED need reviewed_by and decided_at;
        // REJECTED and REVOKED also need a decision_reason_code.
        let decided = matches!(status, "VERIFIED" | "REJECTED" | "REVOKED");
        let reason = matches!(status, "REJECTED" | "REVOKED").then_some("OTHER");
        room.insert_case(
            case_id,
            ORG_1,
            "CX_1",
            status,
            None,
            Some(Utc::now() - ChronoDuration::days(1)),
            decided.then(Utc::now),
            None,
            decided.then(Uuid::new_v4),
            reason,
            2,
        )
        .await;

        let ans = call_multipart(
            &room,
            Some(&cust),
            &format!("/api/app/verification/cases/{case_id}/documents"),
            boundary,
            upload_payload.clone(),
            Some(&format!("IDEM-LOCK-{status}")),
        )
        .await;

        assert!(
            ans.status == StatusCode::BAD_REQUEST || ans.status == StatusCode::CONFLICT,
            "upload must be refused when case is {status}, got {}: {}",
            ans.status,
            ans.text
        );
        assert_eq!(ans.code(), "caseLocked");
    }

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 9: Upload and submit allowed when case in NEEDS_UPDATE
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn upload_and_submit_allowed_when_case_in_needs_update() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();

    // Case placed in NEEDS_UPDATE by reviewer
    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "NEEDS_UPDATE",
        None,
        Some(Utc::now() - ChronoDuration::days(2)),
        Some(Utc::now() - ChronoDuration::days(1)),
        None,
        Some(Uuid::new_v4()),
        Some("DOCUMENT_UNREADABLE"),
        3,
    )
    .await;

    let boundary = "---------------------------c10needsupdateboundary";
    let upload_payload = make_multipart(
        boundary,
        &[("documentType", "NATIONAL_ID")],
        Some(("file", "idcard.png", "image/png", SECRET_DOC_BYTES)),
    );

    // 1. Upload in NEEDS_UPDATE succeeds
    let ans_upload = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        upload_payload,
        Some("IDEM-UPLOAD-NEEDS-UPDATE"),
    )
    .await;
    let upload_data = ans_upload.ok("upload during needs update");
    assert_eq!(upload_data["documentType"].as_str().unwrap(), "NATIONAL_ID");

    // 2. Submit in NEEDS_UPDATE succeeds and moves case to SUBMITTED
    let ans_submit = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/cases/{case_id}/submit"),
        Some((json!({}), "IDEM-SUBMIT-NEEDS-UPDATE")),
        None,
    )
    .await;
    let submit_data = ans_submit.ok("submit during needs update");
    assert_eq!(submit_data["status"].as_str().unwrap(), "SUBMITTED");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 10: Upload refuses document type not accepted by policy
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn upload_refuses_document_type_not_accepted_by_policy() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    let boundary = "---------------------------c10unacceptedtype";
    let upload_payload = make_multipart(
        boundary,
        &[("documentType", "PHONE")], // Policy accepts only PASSPORT and NATIONAL_ID
        Some(("file", "phone.png", "image/png", SECRET_DOC_BYTES)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        upload_payload,
        Some("IDEM-UNACCEPTED-TYPE"),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "uploading unaccepted document type must return 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "documentTypeNotAccepted");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 11: Upload over 15MB is refused with file_service too large sentence
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn upload_over_15mb_is_refused_with_file_service_too_large_sentence() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    let huge_bytes = vec![0u8; 15 * 1024 * 1024 + 1024]; // > 15 MB
    let boundary = "---------------------------c10toolargeboundary";
    let upload_payload = make_multipart(
        boundary,
        &[("documentType", "PASSPORT")],
        Some(("file", "huge.png", "image/png", &huge_bytes)),
    );

    let ans = call_multipart(
        &room,
        Some(&cust),
        &format!("/api/app/verification/cases/{case_id}/documents"),
        boundary,
        upload_payload,
        Some("IDEM-TOO-LARGE"),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "upload over 15MB must return 400: {}",
        ans.text
    );
    assert!(
        ans.text.contains("ไฟล์ใหญ่เกิน 15 MB"),
        "refusal must contain file_service sentence 'ไฟล์ใหญ่เกิน 15 MB', got: {}",
        ans.text
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 12: document_remove succeeds in DRAFT and refuses when case is locked
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn document_remove_succeeds_in_draft_and_refuses_when_case_locked() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-remove"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
        Uuid::new_v4(),
    )
    .await;

    // 1. Remove in DRAFT succeeds
    let ans_remove = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-1")),
        None,
    )
    .await;
    ans_remove.ok("remove document in draft");

    // 2. Put case in SUBMITTED and try removing a document -> caseLocked
    let locked_case_id = Uuid::new_v4();
    let locked_doc_id = Uuid::new_v4();
    room.close_open_cases("CX_1").await;
    room.insert_case(
        locked_case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        None,
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        2,
    )
    .await;
    room.insert_document(
        locked_doc_id,
        ORG_1,
        locked_case_id,
        "PASSPORT",
        Some("ref-doc-locked"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
        Uuid::new_v4(),
    )
    .await;

    let ans_locked_remove = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{locked_doc_id}/remove"),
        Some((json!({}), "IDEM-REMOVE-LOCKED")),
        None,
    )
    .await;

    assert!(
        ans_locked_remove.status == StatusCode::BAD_REQUEST
            || ans_locked_remove.status == StatusCode::CONFLICT,
        "remove on submitted case must return 400 or 409, got {}: {}",
        ans_locked_remove.status,
        ans_locked_remove.text
    );
    assert_eq!(ans_locked_remove.code(), "caseLocked");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 13: case_submit moves DRAFT to SUBMITTED and refuses when documents missing
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn case_submit_moves_draft_to_submitted_and_refuses_when_documents_missing() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    // 1. Submit with 0 documents attached -> refused with documentsMissing
    let ans_submit_empty = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/cases/{case_id}/submit"),
        Some((json!({}), "IDEM-SUBMIT-EMPTY")),
        None,
    )
    .await;
    assert_eq!(
        ans_submit_empty.status,
        StatusCode::BAD_REQUEST,
        "submitting case with no documents must return 400: {}",
        ans_submit_empty.text
    );
    assert_eq!(ans_submit_empty.code(), "documentsMissing");

    // 2. Attach required document
    let doc_id = Uuid::new_v4();
    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-valid"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
        Uuid::new_v4(),
    )
    .await;

    // 3. Submit with document attached -> succeeds and moves to SUBMITTED
    let ans_submit_ok = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/cases/{case_id}/submit"),
        Some((json!({}), "IDEM-SUBMIT-OK")),
        None,
    )
    .await;
    let data = ans_submit_ok.ok("submit case with documents");
    assert_eq!(data["status"].as_str().unwrap(), "SUBMITTED");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 14: my_document_link issues download link for owner only and audits
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn my_document_link_issues_download_link_for_owner_only() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let other_cust = Who::customer("user-cust-2", "CX_2");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id, ORG_1, "CX_1", "DRAFT", None, None, None, None, None, None, 1,
    )
    .await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("ref-doc-owner-link"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
        Uuid::new_v4(),
    )
    .await;

    // 1. Owner requests document link -> receives url and expiresAt
    let ans_ok = call(
        &room,
        Some(&cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/link"),
        Some((json!({}), "IDEM-MY-DOC-LINK")),
        None,
    )
    .await;
    let data = ans_ok.ok("owner link");
    assert!(data["url"].as_str().is_some(), "url must be present");
    assert!(
        data["expiresAt"].as_str().is_some(),
        "expiresAt must be present"
    );

    // 2. Another customer requests the link -> 403 outOfScope
    let ans_other = call(
        &room,
        Some(&other_cust),
        Method::POST,
        &format!("/api/app/verification/documents/{doc_id}/link"),
        Some((json!({}), "IDEM-OTHER-LINK")),
        None,
    )
    .await;
    assert_eq!(
        ans_other.status,
        StatusCode::FORBIDDEN,
        "non-owner must receive 403: {}",
        ans_other.text
    );
    assert_eq!(ans_other.code(), "outOfScope");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 15: my_verification view carries no image bytes and shows metadata and readiness
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn my_verification_view_carries_no_image_bytes_and_shows_metadata_and_readiness() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        None,
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    let checksum = Sha256::digest(SECRET_DOC_BYTES);
    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("secret-passport-storage-ref"),
        &checksum,
        SECRET_DOC_BYTES.len() as i32,
        Uuid::new_v4(),
    )
    .await;

    let ans = call(
        &room,
        Some(&cust),
        Method::GET,
        "/api/v3/views/my_verification",
        None,
        None,
    )
    .await;
    let data = ans.ok("my_verification view");

    // Case metadata
    assert_eq!(data["case"]["status"].as_str().unwrap(), "SUBMITTED");
    assert!(data["readiness"].as_str().is_some());

    // Documents metadata present
    let docs = data["case"]["documents"]
        .as_array()
        .or_else(|| data["documents"].as_array())
        .expect("documents array");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["documentType"].as_str().unwrap(), "PASSPORT");

    // CRITICAL: Ensure NO image bytes, base64 data, or storage refs in payload
    assert!(
        !ans.text.contains("CONFIDENTIAL_CUSTOMER_PASSPORT"),
        "my_verification response must never contain document bytes"
    );
    let raw_b64 = base64_encode(SECRET_DOC_BYTES, false, false);
    assert!(
        !ans.text.contains(&raw_b64),
        "my_verification response must never contain base64 of document"
    );
    assert!(
        !ans.text.contains("secret-passport-storage-ref"),
        "my_verification response must not leak storage object ref"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 16: Idempotency-Key replay returns first answer, different body -> conflict
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn idempotency_key_replay_returns_cached_response_and_conflict_on_different_body() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");
    let key = "IDEM-TEST-C10-IDEMPOTENCY";
    let body_a = json!({"reason": "initial"});
    let body_b = json!({"reason": "modified"});

    // 1. First execution
    let ans_1 = call(
        &room,
        Some(&cust),
        Method::POST,
        "/api/app/verification/cases",
        Some((body_a.clone(), key)),
        None,
    )
    .await;
    let data_1 = ans_1.ok("first execution");

    // 2. Replay with identical key and body -> identical cached answer
    let ans_replay = call(
        &room,
        Some(&cust),
        Method::POST,
        "/api/app/verification/cases",
        Some((body_a, key)),
        None,
    )
    .await;
    let data_replay = ans_replay.ok("replay execution");
    assert_eq!(data_1, data_replay);

    // 3. Replay with different body -> 409 idempotencyConflict
    let ans_conflict = call(
        &room,
        Some(&cust),
        Method::POST,
        "/api/app/verification/cases",
        Some((body_b, key)),
        None,
    )
    .await;
    assert_eq!(
        ans_conflict.status,
        StatusCode::CONFLICT,
        "replaying same key with different body must return 409: {}",
        ans_conflict.text
    );
    assert_eq!(ans_conflict.code(), "idempotencyConflict");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Test 17: No customer command can transition case to reviewer states
// -----------------------------------------------------------------------------
#[actix_web::test]
async fn no_customer_command_can_transition_to_reviewer_state() {
    let room = Room::start().await;
    let cust = Who::customer("user-cust-1", "CX_1");

    // Reviewer states: UNDER_REVIEW, VERIFIED, REJECTED, REVOKED
    let states = [
        "SUBMITTED",
        "UNDER_REVIEW",
        "VERIFIED",
        "REJECTED",
        "EXPIRED",
        "REVOKED",
    ];

    for st in states {
        let case_id = Uuid::new_v4();
        room.close_open_cases("CX_1").await;
        // The table's CHECKs: VERIFIED, REJECTED and REVOKED need reviewed_by and decided_at;
        // REJECTED and REVOKED also need a decision_reason_code.
        let decided = matches!(st, "VERIFIED" | "REJECTED" | "REVOKED");
        let reason = matches!(st, "REJECTED" | "REVOKED").then_some("OTHER");
        room.insert_case(
            case_id,
            ORG_1,
            "CX_1",
            st,
            None,
            Some(Utc::now() - ChronoDuration::days(1)),
            decided.then(Utc::now),
            None,
            decided.then(Uuid::new_v4),
            reason,
            1,
        )
        .await;

        // Submitting any case not in DRAFT or NEEDS_UPDATE must fail
        let ans_submit = call(
            &room,
            Some(&cust),
            Method::POST,
            &format!("/api/app/verification/cases/{case_id}/submit"),
            Some((json!({}), &format!("IDEM-STATE-CHECK-{st}"))),
            None,
        )
        .await;

        assert!(
            ans_submit.status == StatusCode::BAD_REQUEST
                || ans_submit.status == StatusCode::CONFLICT,
            "submitting case in state {st} must be refused, got {}: {}",
            ans_submit.status,
            ans_submit.text
        );
    }

    room.finish().await;
}
