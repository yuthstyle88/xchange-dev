//! VF-7 #1396 — reviewer HTTP API (F02 queue, decision commands, reviewer document link, content route).
//! Design `docs/design/blueprint/verification.md` §4.5, §7, §8, §9.1 row VF-7; ticket #1396 §4, §8, §9.
//!
//! Written before the implementation by the test writer gemini/studio-13, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Behavior & Edge Cases locked by these tests:
//! 1. `every_f02_route_refuses_without_verification_review`:
//!    All 6 commands + 2 views refuse when caller lacks `VerificationReview` (TELLER, ADMIN, EXECUTIVE, CUSTOMER, unsigned).
//! 2. `verification_queue_shows_only_submitted_and_under_review_cases_oldest_first`:
//!    Queue returns only SUBMITTED and UNDER_REVIEW cases in scope, ordered oldest first, with customer display name and identity last 4.
//! 3. `branch_scoped_reviewer_only_sees_their_branch_and_never_app_cases`:
//!    A branch-scoped reviewer does not see app cases (`branch_id IS NULL`) or other branches' cases unless holding org-scoped role (V17).
//! 4. `reviewer_from_another_organisation_is_refused`:
//!    A reviewer from another organisation calling commands or views is refused with `outOfScope` (HTTP 403).
//! 5. `verification_queue_since_parameter_polling`:
//!    `since` equal to current version returns `{changed: false}`; older `since` returns full list with updated version.
//! 6. `f02_view_responses_never_contain_image_bytes_base64_or_full_identity`:
//!    Neither `verification_queue` nor `verification_case` view response contains document image bytes, base64 data, or full identity number (only last 4).
//! 7. `view_verification_case_returns_case_audit_and_document_metadata_only`:
//!    `verification_case?id=` returns case details, audit trail, and document metadata (type, uploaded at, size, state) without bytes.
//! 8. `reviewer_command_start_review_moves_submitted_to_under_review`:
//!    `start-review` moves case from SUBMITTED to UNDER_REVIEW, bumping version.
//! 9. `reviewer_command_approve_requires_future_expires_at_bangkok_timezone`:
//!    Approve with `expiresAt` today or in past (Bangkok timezone) is refused with `validationFailed`; future date succeeds and sets status to VERIFIED.
//! 10. `reviewer_commands_reject_request_update_revoke_succeed_with_s5_reasons`:
//!     Happy path transitions: `reject` (UNDER_REVIEW -> REJECTED), `request-update` (UNDER_REVIEW -> NEEDS_UPDATE), `revoke` (VERIFIED -> REVOKED) with S-5 reason codes.
//! 11. `unlisted_reason_code_is_refused_as_validation_failed`:
//!     Reject, request-update, and revoke commands with reason code outside the S-5 list are refused with `validationFailed` (moved from VF-4).
//! 12. `stale_version_when_expected_version_does_not_match`:
//!     Reviewer command with mismatched `expectedVersion` returns `staleVersion`.
//! 13. `idempotency_key_replay_returns_cached_response_and_conflict_on_different_body`:
//!     Replaying same key and body returns cached response; different body with same key returns `idempotencyConflict` (HTTP 409).
//! 14. `document_link_command_and_content_route_lifecycle`:
//!     Link command returns `{url, expiresAt}`; content route serves bytes with required headers:
//!     `Cache-Control: private, no-store`, `X-Content-Type-Options: nosniff`, `Content-Disposition: attachment`.
//! 15. `content_token_reused_by_another_session_is_refused`:
//!     Content token bound to session A is refused when fetched with session B (`linkRefused`).
//! 16. `expired_content_token_is_refused`:
//!     Expired content token is refused (`linkExpired`).
//!
//! Amended by #1943's ticket author (claude/c4d72c, 2026-10-15; design `customer-app.md` §9.2): the harness seeds a
//! real `user_sessions` row for every seeded user (`hash` = sha256 hex of the `session-{user_id}` cookie), because the
//! link command no longer falls back to a made-up session. Tests 14–16 are otherwise unchanged.

mod common;

use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::header::HeaderMap;
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use std::future::Future;
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::document_store::DocumentStore;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys, Sealed};
use xc_verification::application::access::download_link::{DownloadLink, LINK_LIFETIME};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::document_store::FsDocumentStore;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const ORG_2: &str = "00000000-0000-0000-0000-000000000002";
const BRANCH_MAIN: &str = "MAIN";
const BRANCH_B2: &str = "BRANCH_B2";

const SECRET_DOC_BYTES: &[u8] = b"CONFIDENTIAL_PASSPORT_IMAGE_RAW_BYTES_FOR_VF7_TESTING";
const SECRET_IDENTITY_FULL: &str = "AA1231234";
const SECRET_IDENTITY_LAST4: &str = "1234";

const S5_REASON_CODES: [&str; 6] = [
    "DOCUMENT_UNREADABLE",
    "DOCUMENT_EXPIRED",
    "DETAILS_MISMATCH",
    "DOCUMENT_NOT_ACCEPTED",
    "SUSPECTED_FRAUD",
    "OTHER",
];

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

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[allow(clippy::manual_is_multiple_of, clippy::chunks_exact_to_as_chunks)]
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks_exact(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        bytes.push((hi << 4) | lo);
    }
    Some(bytes)
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

    fn branch_reviewer(user_id: &str, branch: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::VerificationReviewer,
            branch: Some(branch.to_owned()),
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

    fn other_org_reviewer(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::VerificationReviewer,
            branch: None,
            customer_id: None,
            org_id: ORG_2.to_owned(),
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

    #[allow(dead_code)]
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

    fn rate_manager(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::RateManager,
            branch: None,
            customer_id: None,
            org_id: ORG_1.to_owned(),
            session: format!("session-{user_id}"),
        }
    }

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

        let schema = unique("vf7");
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
        // Seed ORG_2 and its policy version 1 (with created_at passed, per ticket instructions)
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

        // Seed branches
        sqlx::query(
            "INSERT INTO branches (id, organization_id, name) VALUES \
             ($1, $2, 'Main Branch'), \
             ($3, $2, 'Branch B2') \
             ON CONFLICT (id) DO UPDATE SET organization_id = EXCLUDED.organization_id",
        )
        .bind(BRANCH_MAIN)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(BRANCH_B2)
        .execute(&self.pool)
        .await
        .expect("seed branches");

        // Seed users
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ('11111111-1111-1111-1111-111111111111', $1, 'Org Reviewer', 'ACTIVE', 'seed'), \
             ('22222222-2222-2222-2222-222222222222', $1, 'Branch Reviewer', 'ACTIVE', 'seed'), \
             ('33333333-3333-3333-3333-333333333333', $1, 'Teller User', 'ACTIVE', 'seed'), \
             ('44444444-4444-4444-4444-444444444444', $2, 'Org2 Reviewer', 'ACTIVE', 'seed') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(Uuid::from_str(ORG_2).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed users");

        // #1943 (claude/c4d72c, the ticket author): every seeded user is signed in for real. Their cookie is the
        // `session-{user_id}` each `Who` sends, and `user_sessions.hash` is its sha256 hex, as `SessionService`
        // stores it, so the link command binds a link to the caller's own live session (no fallback).
        for (user, org) in [
            ("11111111-1111-1111-1111-111111111111", ORG_1),
            ("22222222-2222-2222-2222-222222222222", ORG_1),
            ("33333333-3333-3333-3333-333333333333", ORG_1),
            ("44444444-4444-4444-4444-444444444444", ORG_2),
        ] {
            let hash: String = Sha256::digest(format!("session-{user}").as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            sqlx::query(
                "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
                 VALUES ($1, $2::uuid, 1, now() + interval '8 hours', $3) ON CONFLICT (hash) DO NOTHING",
            )
            .bind(hash)
            .bind(user)
            .bind(Uuid::from_str(org).unwrap())
            .execute(&self.pool)
            .await
            .expect("seed a live session");
        }

        // Seed customers
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) VALUES \
             ('CX_1', $1, 'CX_1', 'Somchai Prasert', 'LIVE', now()), \
             ('CX_2', $1, 'CX_2', 'Somsri Sukjai', 'LIVE', now()), \
             ('CX_3', $1, 'CX_3', 'Pornchai Dee', 'LIVE', now()), \
             ('CX_4', $1, 'CX_4', 'Mali Thong', 'LIVE', now()), \
             ('CX_5', $1, 'CX_5', 'Anan Sak', 'LIVE', now()), \
             ('CX_6', $1, 'CX_6', 'Kanda Sri', 'LIVE', now()), \
             ('CX_7', $1, 'CX_7', 'Wichai Boon', 'LIVE', now()), \
             ('CX_8', $1, 'CX_8', 'Napa Charoen', 'LIVE', now()), \
             ('CX_ORG2', $2, 'CX_ORG2', 'Foreign Customer', 'LIVE', now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(Uuid::from_str(ORG_2).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed customers");

        // Seed customer identity for CX_1 (PASSPORT sealed JSON payload with AAD {customer_id}PASSPORT per customer service)
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

    async fn today_bangkok(&self) -> NaiveDate {
        sqlx::query_scalar::<_, NaiveDate>("SELECT (now() AT TIME ZONE 'Asia/Bangkok')::date")
            .fetch_one(&self.pool)
            .await
            .expect("today in Bangkok")
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
        .bind(reviewed_by)
        .bind(decision_reason_code)
        .bind(version)
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
        uploaded_by: Uuid,
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
        .bind(uploaded_by)
        .execute(&self.pool)
        .await
        .expect("insert verification document");
    }

    async fn insert_audit_event(
        &self,
        actor_id: &str,
        actor_role: &str,
        action: &str,
        object_ref: &str,
        before: Option<Value>,
        after: Option<Value>,
    ) {
        sqlx::query(
            "INSERT INTO audit_events (actor_id, actor_role, scope, branch_id, action, object_ref, before_redacted, after_redacted, at) \
             VALUES ($1, $2, 'ORGANIZATION', NULL, $3, $4, $5, $6, now())",
        )
        .bind(actor_id)
        .bind(actor_role)
        .bind(action)
        .bind(object_ref)
        .bind(before)
        .bind(after)
        .execute(&self.pool)
        .await
        .expect("insert audit event");
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

// =================================================================================================
// 1. Refusals: Every F02 route refuses without VerificationReview
// =================================================================================================
#[actix_web::test]
async fn every_f02_route_refuses_without_verification_review() {
    let room = Room::start().await;
    let unauth_callers = [
        (
            "teller",
            Who::teller("33333333-3333-3333-3333-333333333333", BRANCH_MAIN),
        ),
        ("admin", Who::admin("55555555-5555-5555-5555-555555555555")),
        // Under RBAC §11.2, Executive holds VerificationReview (Documents: Full); RateManager does not (Documents: View).
        (
            "rate_manager",
            Who::rate_manager("66666666-6666-6666-6666-666666666666"),
        ),
        (
            "customer",
            Who::customer("77777777-7777-7777-7777-777777777777", "CX_1"),
        ),
    ];
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    let routes = [
        (
            Method::POST,
            format!("/api/operations/verification/cases/{case_id}/start-review"),
            Some((json!({"expectedVersion": 1}), "IDEM-REFUSE-1")),
        ),
        (
            Method::POST,
            format!("/api/operations/verification/cases/{case_id}/approve"),
            Some((
                json!({"expectedVersion": 1, "expiresAt": "2030-01-01"}),
                "IDEM-REFUSE-2",
            )),
        ),
        (
            Method::POST,
            format!("/api/operations/verification/cases/{case_id}/reject"),
            Some((
                json!({"expectedVersion": 1, "reasonCode": "DOCUMENT_EXPIRED"}),
                "IDEM-REFUSE-3",
            )),
        ),
        (
            Method::POST,
            format!("/api/operations/verification/cases/{case_id}/request-update"),
            Some((
                json!({"expectedVersion": 1, "reasonCode": "DOCUMENT_UNREADABLE"}),
                "IDEM-REFUSE-4",
            )),
        ),
        (
            Method::POST,
            format!("/api/operations/verification/cases/{case_id}/revoke"),
            Some((
                json!({"expectedVersion": 1, "reasonCode": "SUSPECTED_FRAUD"}),
                "IDEM-REFUSE-5",
            )),
        ),
        (
            Method::POST,
            format!("/api/operations/verification/documents/{doc_id}/link"),
            Some((json!({"expectedVersion": 1}), "IDEM-REFUSE-6")),
        ),
        (
            Method::GET,
            "/api/v3/views/verification_queue".to_string(),
            None,
        ),
        (
            Method::GET,
            format!("/api/v3/views/verification_case?id={case_id}"),
            None,
        ),
    ];

    for (method, uri, body) in routes {
        // Unauthenticated call
        let unauth = call(&room, None, method.clone(), &uri, body.clone(), None).await;
        assert_eq!(
            unauth.status,
            StatusCode::UNAUTHORIZED,
            "unauthenticated call to {uri} must answer 401"
        );

        // Unauthorized callers (TELLER, ADMIN, EXECUTIVE, CUSTOMER lack VerificationReview)
        for (label, who) in &unauth_callers {
            let unauth_caller =
                call(&room, Some(who), method.clone(), &uri, body.clone(), None).await;
            assert!(
                unauth_caller.status == StatusCode::FORBIDDEN
                    || unauth_caller.status == StatusCode::BAD_REQUEST,
                "{label} call to {uri} must be refused with 403 or 400: got {}",
                unauth_caller.status
            );
            let code = unauth_caller.code();
            assert!(
                code == "outOfScope" || code == "roleNotAllowedHere",
                "{label} call to {uri} code was '{code}'"
            );
        }
    }

    room.finish().await;
}

// =================================================================================================
// 2. Queue view: Only SUBMITTED and UNDER_REVIEW cases, oldest first
// =================================================================================================
#[actix_web::test]
async fn verification_queue_shows_only_submitted_and_under_review_cases_oldest_first() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");

    let t0 = Utc::now() - ChronoDuration::hours(5);
    let t1 = Utc::now() - ChronoDuration::hours(3);
    let t2 = Utc::now() - ChronoDuration::hours(1);

    let case_submitted = Uuid::new_v4();
    let case_under_review = Uuid::new_v4();
    let case_draft = Uuid::new_v4();
    let case_verified = Uuid::new_v4();
    let case_rejected = Uuid::new_v4();

    // Only SUBMITTED and UNDER_REVIEW belong in the queue
    room.insert_case(
        case_under_review,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(t1),
        None,
        None,
        None,
        None,
        1,
    )
    .await;
    room.insert_case(
        case_submitted,
        ORG_1,
        "CX_2",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(t2),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    // These should NEVER appear in the queue
    room.insert_case(
        case_draft,
        ORG_1,
        "CX_3",
        "DRAFT",
        Some(BRANCH_MAIN),
        None,
        None,
        None,
        None,
        None,
        1,
    )
    .await;
    room.insert_case(
        case_verified,
        ORG_1,
        "CX_4",
        "VERIFIED",
        Some(BRANCH_MAIN),
        Some(t0),
        Some(t0),
        Some(Utc::now() + ChronoDuration::days(365)),
        Some(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()),
        None,
        2,
    )
    .await;
    room.insert_case(
        case_rejected,
        ORG_1,
        "CX_5",
        "REJECTED",
        Some(BRANCH_MAIN),
        Some(t0),
        Some(t0),
        None,
        Some(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()),
        Some("DOCUMENT_EXPIRED"),
        2,
    )
    .await;

    let ans = call(
        &room,
        Some(&reviewer),
        Method::GET,
        "/api/v3/views/verification_queue",
        None,
        None,
    )
    .await;
    let data = ans.ok("verification_queue");

    let list = data
        .as_array()
        .or_else(|| data["data"].as_array())
        .or_else(|| data["cases"].as_array())
        .expect("queue list array");

    assert_eq!(list.len(), 2, "only 2 cases must be in queue: {}", ans.text);

    // Oldest first: t1 (UNDER_REVIEW) then t2 (SUBMITTED)
    assert_eq!(
        list[0]["id"].as_str().unwrap(),
        case_under_review.to_string()
    );
    assert_eq!(list[0]["status"].as_str().unwrap(), "UNDER_REVIEW");
    assert_eq!(list[1]["id"].as_str().unwrap(), case_submitted.to_string());
    assert_eq!(list[1]["status"].as_str().unwrap(), "SUBMITTED");

    // Check fields on each row
    assert!(list[0].get("submittedAt").is_some());
    assert!(list[0].get("customerDisplayName").is_some());
    assert!(list[0].get("identityLast4").is_some());

    room.finish().await;
}

// =================================================================================================
// 3. Queue view: Branch-scoped reviewer sees only their branch (never app cases)
// =================================================================================================
#[actix_web::test]
async fn branch_scoped_reviewer_only_sees_their_branch_and_never_app_cases() {
    let room = Room::start().await;
    let branch_reviewer = Who::branch_reviewer("22222222-2222-2222-2222-222222222222", BRANCH_MAIN);
    let org_reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");

    let app_case = Uuid::new_v4();
    let main_case = Uuid::new_v4();
    let b2_case = Uuid::new_v4();

    // App case: branch_id is NULL
    room.insert_case(
        app_case,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        None,
        Some(Utc::now() - ChronoDuration::hours(3)),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    // Branch MAIN case
    room.insert_case(
        main_case,
        ORG_1,
        "CX_2",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now() - ChronoDuration::hours(2)),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    // Branch B2 case
    room.insert_case(
        b2_case,
        ORG_1,
        "CX_3",
        "SUBMITTED",
        Some(BRANCH_B2),
        Some(Utc::now() - ChronoDuration::hours(1)),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    // 1. Branch-scoped reviewer for MAIN must only see main_case
    let ans_branch = call(
        &room,
        Some(&branch_reviewer),
        Method::GET,
        "/api/v3/views/verification_queue",
        None,
        None,
    )
    .await;
    let data_branch = ans_branch.ok("branch reviewer queue");
    let list_branch = data_branch
        .as_array()
        .or_else(|| data_branch["data"].as_array())
        .or_else(|| data_branch["cases"].as_array())
        .expect("queue list array");

    let ids_branch: Vec<&str> = list_branch
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert_eq!(
        ids_branch,
        vec![main_case.to_string().as_str()],
        "branch reviewer must only see MAIN case: {}",
        ans_branch.text
    );

    // 2. Org-wide reviewer sees all 3 cases
    let ans_org = call(
        &room,
        Some(&org_reviewer),
        Method::GET,
        "/api/v3/views/verification_queue",
        None,
        None,
    )
    .await;
    let data_org = ans_org.ok("org reviewer queue");
    let list_org = data_org
        .as_array()
        .or_else(|| data_org["data"].as_array())
        .or_else(|| data_org["cases"].as_array())
        .expect("queue list array");
    assert_eq!(
        list_org.len(),
        3,
        "org reviewer must see all cases: {}",
        ans_org.text
    );

    room.finish().await;
}

// =================================================================================================
// 4. Cross-org: Reviewer from another organisation is refused
// =================================================================================================
#[actix_web::test]
async fn reviewer_from_another_organisation_is_refused() {
    let room = Room::start().await;
    let other_reviewer = Who::other_org_reviewer("44444444-4444-4444-4444-444444444444");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    // Command attempt across organisation
    let ans_cmd = call(
        &room,
        Some(&other_reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/start-review"),
        Some((json!({"expectedVersion": 1}), "IDEM-CROSS-ORG")),
        None,
    )
    .await;
    assert_eq!(
        ans_cmd.status,
        StatusCode::FORBIDDEN,
        "cross-org command must be 403: {}",
        ans_cmd.text
    );
    assert_eq!(ans_cmd.code(), "outOfScope");

    // Case view attempt across organisation
    let ans_view = call(
        &room,
        Some(&other_reviewer),
        Method::GET,
        &format!("/api/v3/views/verification_case?id={case_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        ans_view.status,
        StatusCode::FORBIDDEN,
        "cross-org case view must be 403: {}",
        ans_view.text
    );
    assert_eq!(ans_view.code(), "outOfScope");

    room.finish().await;
}

// =================================================================================================
// 5. Polling with `since`: unchanged returns changed: false; older returns full list
// =================================================================================================
#[actix_web::test]
async fn verification_queue_since_parameter_polling() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    // 1. Initial query without since
    let ans_init = call(
        &room,
        Some(&reviewer),
        Method::GET,
        "/api/v3/views/verification_queue",
        None,
        None,
    )
    .await;
    let init_data = ans_init.ok("initial queue query");
    let version = init_data["version"]
        .as_i64()
        .expect("queue response carries version");

    // 2. Query with since = current version -> changed: false
    let ans_unchanged = call(
        &room,
        Some(&reviewer),
        Method::GET,
        &format!("/api/v3/views/verification_queue?since={version}"),
        None,
        None,
    )
    .await;
    let unchanged_data = ans_unchanged.ok("unchanged queue poll");
    assert_eq!(
        unchanged_data["changed"].as_bool(),
        Some(false),
        "queue poll with current version must return changed: false: {}",
        ans_unchanged.text
    );

    // 3. Query with older since (e.g. 0) -> changed: true and full list
    let ans_changed = call(
        &room,
        Some(&reviewer),
        Method::GET,
        "/api/v3/views/verification_queue?since=0",
        None,
        None,
    )
    .await;
    let changed_data = ans_changed.ok("older since queue poll");
    assert_ne!(
        changed_data["changed"].as_bool(),
        Some(false),
        "queue poll with older since must return changed: true: {}",
        ans_changed.text
    );
    let list = changed_data
        .as_array()
        .or_else(|| changed_data["data"].as_array())
        .or_else(|| changed_data["cases"].as_array())
        .expect("cases array");
    assert_eq!(list.len(), 1);

    room.finish().await;
}

// =================================================================================================
// 6. Security & Privacy: No view returns image bytes, base64, or full identity
// =================================================================================================
#[actix_web::test]
async fn f02_view_responses_never_contain_image_bytes_base64_or_full_identity() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "NATIONAL_ID",
        Some("test-object-ref-1"),
        &Sha256::digest(SECRET_DOC_BYTES)[..],
        SECRET_DOC_BYTES.len() as i32,
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
    )
    .await;

    let raw_secret_str = std::str::from_utf8(SECRET_DOC_BYTES).unwrap();
    let b64_std = base64_encode(SECRET_DOC_BYTES, false, true);
    let b64_std_unpadded = base64_encode(SECRET_DOC_BYTES, false, false);
    let b64_url = base64_encode(SECRET_DOC_BYTES, true, true);
    let b64_url_unpadded = base64_encode(SECRET_DOC_BYTES, true, false);

    // 1. Check verification_queue view response
    let queue_ans = call(
        &room,
        Some(&reviewer),
        Method::GET,
        "/api/v3/views/verification_queue",
        None,
        None,
    )
    .await;
    let queue_text = queue_ans.ok("verification_queue text").to_string();

    assert!(
        !queue_text.contains(SECRET_IDENTITY_FULL),
        "queue view must NOT leak full identity: {queue_text}"
    );
    assert!(
        queue_text.contains(SECRET_IDENTITY_LAST4),
        "queue view must contain last 4 of identity: {queue_text}"
    );
    assert!(
        !queue_text.contains(raw_secret_str),
        "queue view must NOT contain raw document bytes: {queue_text}"
    );
    assert!(
        !queue_text.contains(&b64_std) && !queue_text.contains(&b64_std_unpadded),
        "queue view must NOT contain standard base64 document bytes: {queue_text}"
    );
    assert!(
        !queue_text.contains(&b64_url) && !queue_text.contains(&b64_url_unpadded),
        "queue view must NOT contain URL-safe base64 document bytes: {queue_text}"
    );

    // 2. Check verification_case view response
    let case_ans = call(
        &room,
        Some(&reviewer),
        Method::GET,
        &format!("/api/v3/views/verification_case?id={case_id}"),
        None,
        None,
    )
    .await;
    let case_text = case_ans.ok("verification_case text").to_string();

    assert!(
        !case_text.contains(SECRET_IDENTITY_FULL),
        "case view must NOT leak full identity: {case_text}"
    );
    assert!(
        !case_text.contains(raw_secret_str),
        "case view must NOT contain raw document bytes: {case_text}"
    );
    assert!(
        !case_text.contains(&b64_std) && !case_text.contains(&b64_std_unpadded),
        "case view must NOT contain standard base64 document bytes: {case_text}"
    );
    assert!(
        !case_text.contains(&b64_url) && !case_text.contains(&b64_url_unpadded),
        "case view must NOT contain URL-safe base64 document bytes: {case_text}"
    );

    room.finish().await;
}

// =================================================================================================
// 7. Case view: Returns case, its audit rows, document metadata (never bytes)
// =================================================================================================
#[actix_web::test]
async fn view_verification_case_returns_case_audit_and_document_metadata_only() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now() - ChronoDuration::hours(2)),
        None,
        None,
        None,
        None,
        2,
    )
    .await;

    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("test-object-ref-doc-1"),
        &Sha256::digest(SECRET_DOC_BYTES)[..],
        SECRET_DOC_BYTES.len() as i32,
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
    )
    .await;

    let other_case_id = Uuid::new_v4();
    room.insert_case(
        other_case_id,
        ORG_1,
        "CX_2",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now() - ChronoDuration::hours(3)),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    room.insert_audit_event(
        "11111111-1111-1111-1111-111111111111",
        "VERIFICATION_REVIEWER",
        "verification_case.start_review",
        &case_id.to_string(),
        Some(json!({"status": "SUBMITTED"})),
        Some(json!({"status": "UNDER_REVIEW"})),
    )
    .await;

    room.insert_audit_event(
        "11111111-1111-1111-1111-111111111111",
        "VERIFICATION_REVIEWER",
        "verification_case.start_review",
        &other_case_id.to_string(),
        Some(json!({"status": "SUBMITTED"})),
        Some(json!({"status": "UNDER_REVIEW"})),
    )
    .await;

    let ans = call(
        &room,
        Some(&reviewer),
        Method::GET,
        &format!("/api/v3/views/verification_case?id={case_id}"),
        None,
        None,
    )
    .await;
    let data = ans.ok("verification_case view");

    // Assert case metadata
    let c = if data.get("case").is_some() {
        &data["case"]
    } else {
        data
    };
    assert_eq!(c["id"].as_str().unwrap(), case_id.to_string());
    assert_eq!(c["status"].as_str().unwrap(), "UNDER_REVIEW");
    assert_eq!(c["version"].as_i64().unwrap(), 2);

    // Assert audit trail present and exactly 1 event for this case
    let audit = data["audit"]
        .as_array()
        .or_else(|| data["auditEvents"].as_array())
        .expect("audit rows array");
    assert_eq!(
        audit.len(),
        1,
        "audit rows must contain exactly 1 event for this case"
    );
    let obj_ref = audit[0]["objectRef"]
        .as_str()
        .or_else(|| audit[0]["object_ref"].as_str())
        .expect("objectRef field must be present on audit row");
    assert_eq!(
        obj_ref,
        case_id.to_string(),
        "audit row must match this case_id"
    );

    // Assert document metadata present and contains no bytes
    let docs = data["documents"].as_array().expect("documents metadata");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["id"].as_str().unwrap(), doc_id.to_string());
    assert_eq!(docs[0]["documentType"].as_str().unwrap(), "PASSPORT");
    assert_eq!(docs[0]["mime"].as_str().unwrap(), "image/png");
    assert_eq!(
        docs[0]["sizeBytes"].as_i64().unwrap(),
        SECRET_DOC_BYTES.len() as i64
    );
    assert!(
        docs[0].get("bytes").is_none(),
        "document metadata must never contain file bytes"
    );

    room.finish().await;
}

// =================================================================================================
// 8. Command: Start review moves SUBMITTED to UNDER_REVIEW
// =================================================================================================
#[actix_web::test]
async fn reviewer_command_start_review_moves_submitted_to_under_review() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    let ans = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/start-review"),
        Some((json!({"expectedVersion": 1}), "IDEM-START-1")),
        None,
    )
    .await;
    let data = ans.ok("start-review response");

    assert_eq!(data["status"].as_str().unwrap(), "UNDER_REVIEW");
    assert_eq!(data["version"].as_i64().unwrap(), 2);

    room.finish().await;
}

// =================================================================================================
// 9. Command: Approve requires expiresAt after today (Asia/Bangkok timezone)
// =================================================================================================
#[actix_web::test]
async fn reviewer_command_approve_requires_future_expires_at_bangkok_timezone() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    let today_bkk = room.today_bangkok().await;
    let yesterday_bkk = today_bkk - ChronoDuration::days(1);
    let future_date = today_bkk + ChronoDuration::days(365);

    // 1. Approve with today in Bangkok timezone -> validationFailed
    let ans_today = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/approve"),
        Some((
            json!({"expectedVersion": 1, "expiresAt": today_bkk.to_string()}),
            "IDEM-APP-TODAY",
        )),
        None,
    )
    .await;
    assert_eq!(
        ans_today.status,
        StatusCode::BAD_REQUEST,
        "approve with today must fail: {}",
        ans_today.text
    );
    assert_eq!(ans_today.code(), "validationFailed");

    // 2. Approve with yesterday in Bangkok timezone -> validationFailed
    let ans_past = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/approve"),
        Some((
            json!({"expectedVersion": 1, "expiresAt": yesterday_bkk.to_string()}),
            "IDEM-APP-PAST",
        )),
        None,
    )
    .await;
    assert_eq!(
        ans_past.status,
        StatusCode::BAD_REQUEST,
        "approve with past date must fail: {}",
        ans_past.text
    );
    assert_eq!(ans_past.code(), "validationFailed");

    // 3. Approve with future date -> 200 OK and status VERIFIED
    let ans_future = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/approve"),
        Some((
            json!({"expectedVersion": 1, "expiresAt": future_date.to_string()}),
            "IDEM-APP-FUTURE",
        )),
        None,
    )
    .await;
    let data = ans_future.ok("approve future");
    assert_eq!(data["status"].as_str().unwrap(), "VERIFIED");
    assert!(data.get("expiresAt").is_some());

    let stored_expires_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT expires_at FROM verification_cases WHERE id = $1")
            .bind(case_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch expires_at");
    assert!(
        stored_expires_at.is_some(),
        "stored verification_cases.expires_at must be set"
    );

    room.finish().await;
}

// =================================================================================================
// 10. Commands: Reject, request-update, revoke succeed with S-5 reason codes
// =================================================================================================
#[actix_web::test]
async fn reviewer_commands_reject_request_update_revoke_succeed_with_s5_reasons() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");

    let case_rej = Uuid::new_v4();
    let case_upd = Uuid::new_v4();
    let case_rev = Uuid::new_v4();

    room.insert_case(
        case_rej,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;
    room.insert_case(
        case_upd,
        ORG_1,
        "CX_2",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;
    room.insert_case(
        case_rev,
        ORG_1,
        "CX_1",
        "VERIFIED",
        Some(BRANCH_MAIN),
        Some(Utc::now() - ChronoDuration::days(10)),
        Some(Utc::now() - ChronoDuration::days(10)),
        Some(Utc::now() + ChronoDuration::days(300)),
        Some(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()),
        None,
        1,
    )
    .await;

    // 1. Reject succeeds with each of the six S-5 reason codes
    for (i, reason) in S5_REASON_CODES.iter().enumerate() {
        let case_id = Uuid::new_v4();
        let cust_id = format!("CX_S5_{i}");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2, $1, 'Customer', 'LIVE', now()) ON CONFLICT DO NOTHING",
        )
        .bind(&cust_id)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .execute(&room.pool)
        .await
        .expect("seed s5 customer");

        room.insert_case(
            case_id,
            ORG_1,
            &cust_id,
            "UNDER_REVIEW",
            Some(BRANCH_MAIN),
            Some(Utc::now()),
            None,
            None,
            None,
            None,
            1,
        )
        .await;

        let ans = call(
            &room,
            Some(&reviewer),
            Method::POST,
            &format!("/api/operations/verification/cases/{case_id}/reject"),
            Some((
                json!({"expectedVersion": 1, "reasonCode": reason}),
                &format!("IDEM-REJ-S5-{i}"),
            )),
            None,
        )
        .await;
        let data = ans.ok("reject with s5 reason");
        assert_eq!(data["status"].as_str().unwrap(), "REJECTED");
        assert_eq!(data["decisionReasonCode"].as_str().unwrap(), *reason);
    }

    // 2. Request update
    let ans_upd = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_upd}/request-update"),
        Some((
            json!({"expectedVersion": 1, "reasonCode": "DOCUMENT_UNREADABLE"}),
            "IDEM-UPD",
        )),
        None,
    )
    .await;
    let data_upd = ans_upd.ok("request-update command");
    assert_eq!(data_upd["status"].as_str().unwrap(), "NEEDS_UPDATE");
    assert_eq!(
        data_upd["decisionReasonCode"].as_str().unwrap(),
        "DOCUMENT_UNREADABLE"
    );

    // 3. Revoke
    let ans_rev = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_rev}/revoke"),
        Some((
            json!({"expectedVersion": 1, "reasonCode": "SUSPECTED_FRAUD"}),
            "IDEM-REV",
        )),
        None,
    )
    .await;
    let data_rev = ans_rev.ok("revoke command");
    assert_eq!(data_rev["status"].as_str().unwrap(), "REVOKED");
    assert_eq!(
        data_rev["decisionReasonCode"].as_str().unwrap(),
        "SUSPECTED_FRAUD"
    );

    room.finish().await;
}

// =================================================================================================
// 11. Command: Unlisted reason code is refused with validationFailed (Moved from VF-4)
// =================================================================================================
#[actix_web::test]
async fn unlisted_reason_code_is_refused_as_validation_failed() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    let invalid_reasons = [
        "NOT_A_VALID_REASON",
        "CUSTOM_REASON",
        "document_unreadable", // lower-case invalid
        "FRAUD",
        "",
    ];

    for r in invalid_reasons {
        let ans = call(
            &room,
            Some(&reviewer),
            Method::POST,
            &format!("/api/operations/verification/cases/{case_id}/reject"),
            Some((
                json!({"expectedVersion": 1, "reasonCode": r}),
                &format!("IDEM-UNLISTED-{r}"),
            )),
            None,
        )
        .await;

        assert_eq!(
            ans.status,
            StatusCode::BAD_REQUEST,
            "unlisted reason '{r}' must be refused: {}",
            ans.text
        );
        assert_eq!(ans.code(), "validationFailed");
    }

    room.finish().await;
}

// =================================================================================================
// 12. Command: Stale version is refused
// =================================================================================================
#[actix_web::test]
async fn stale_version_when_expected_version_does_not_match() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();

    // Case is at version 2, SUBMITTED (valid status for start-review, but expectedVersion is stale)
    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        2,
    )
    .await;

    // Caller provides expectedVersion 1 (stale)
    let ans = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/start-review"),
        Some((json!({"expectedVersion": 1}), "IDEM-STALE-1")),
        None,
    )
    .await;

    assert!(
        ans.status == StatusCode::CONFLICT || ans.status == StatusCode::BAD_REQUEST,
        "stale version must fail: {}",
        ans.text
    );
    assert_eq!(ans.code(), "staleVersion");

    room.finish().await;
}

// =================================================================================================
// 13. Idempotency: Replay returns first answer, different body returns conflict
// =================================================================================================
#[actix_web::test]
async fn idempotency_key_replay_returns_cached_response_and_conflict_on_different_body() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    let key = "IDEM-REPLAY-TEST-KEY-1";
    let body_a = json!({"expectedVersion": 1});
    let body_b = json!({"expectedVersion": 2});

    // 1. First execution
    let ans_1 = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/start-review"),
        Some((body_a.clone(), key)),
        None,
    )
    .await;
    let data_1 = ans_1.ok("first execution");

    // 2. Replay with identical key and body -> cached identical answer
    let ans_replay = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/start-review"),
        Some((body_a, key)),
        None,
    )
    .await;
    let data_replay = ans_replay.ok("replay execution");
    assert_eq!(data_1, data_replay, "replay must return identical response");

    // 3. Different body with same key -> idempotencyConflict (409)
    let ans_conflict = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/cases/{case_id}/start-review"),
        Some((body_b, key)),
        None,
    )
    .await;
    assert_eq!(
        ans_conflict.status,
        StatusCode::CONFLICT,
        "different body must return 409: {}",
        ans_conflict.text
    );
    assert_eq!(ans_conflict.code(), "idempotencyConflict");

    room.finish().await;
}

// =================================================================================================
// 14. Document Link & Content route: Link generation, content serving, and security headers
// =================================================================================================
#[actix_web::test]
async fn document_link_command_and_content_route_lifecycle() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("test-object-ref-doc-content"),
        &Sha256::digest(SECRET_DOC_BYTES)[..],
        SECRET_DOC_BYTES.len() as i32,
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
    )
    .await;

    // 1. Issue link
    let ans_link = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/documents/{doc_id}/link"),
        Some((json!({"expectedVersion": 1}), "IDEM-LINK-1")),
        None,
    )
    .await;
    let data_link = ans_link.ok("issue link");

    let url = data_link["url"]
        .as_str()
        .expect("link response carries url");
    assert!(
        url.starts_with("/api/verification/content/"),
        "link url must start with /api/verification/content/: {url}"
    );
    assert!(data_link.get("expiresAt").is_some());

    // 2. Fetch content with the reviewer's session
    let ans_content = call(&room, Some(&reviewer), Method::GET, url, None, None).await;
    assert_eq!(
        ans_content.status,
        StatusCode::OK,
        "content route must answer 200 OK: {}",
        ans_content.text
    );
    assert_eq!(ans_content.bytes, SECRET_DOC_BYTES);

    // 3. Assert V10 required headers
    let cache_control = ans_content
        .headers
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cache_control.contains("private") && cache_control.contains("no-store"),
        "Cache-Control must be private, no-store: {cache_control}"
    );

    let nosniff = ans_content
        .headers
        .get("x-content-type-options")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(nosniff, "nosniff");

    let disp = ans_content
        .headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        disp.contains("inline"),
        "Content-Disposition must be inline for an image: {disp}"
    );

    let ctype = ans_content
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(ctype, "image/png");

    room.finish().await;
}

// =================================================================================================
// 15. Content route: Token reused by another session is refused (S-3)
// =================================================================================================
#[actix_web::test]
async fn content_token_reused_by_another_session_is_refused() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let other_reviewer = Who::branch_reviewer("22222222-2222-2222-2222-222222222222", BRANCH_MAIN);
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("test-object-ref-session-reuse"),
        &Sha256::digest(SECRET_DOC_BYTES)[..],
        SECRET_DOC_BYTES.len() as i32,
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
    )
    .await;

    // Issue link for reviewer A
    let ans_link = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/documents/{doc_id}/link"),
        Some((json!({"expectedVersion": 1}), "IDEM-LINK-S3")),
        None,
    )
    .await;
    let data_link = ans_link.ok("issue link");
    let url = data_link["url"].as_str().unwrap();

    // Fetch link using other reviewer's session
    let ans_other = call(&room, Some(&other_reviewer), Method::GET, url, None, None).await;

    assert!(
        ans_other.status == StatusCode::FORBIDDEN || ans_other.status == StatusCode::BAD_REQUEST,
        "token reused by another session must be refused: {}",
        ans_other.text
    );
    assert_eq!(ans_other.code(), "linkRefused");

    room.finish().await;
}

// =================================================================================================
// 16. Content route: Expired token is refused
// =================================================================================================
#[actix_web::test]
async fn expired_content_token_is_refused() {
    let room = Room::start().await;
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let case_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    room.insert_case(
        case_id,
        ORG_1,
        "CX_1",
        "UNDER_REVIEW",
        Some(BRANCH_MAIN),
        Some(Utc::now()),
        None,
        None,
        None,
        None,
        1,
    )
    .await;

    room.insert_document(
        doc_id,
        ORG_1,
        case_id,
        "PASSPORT",
        Some("test-object-ref-expired"),
        &Sha256::digest(SECRET_DOC_BYTES)[..],
        SECRET_DOC_BYTES.len() as i32,
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
    )
    .await;

    // 1. Issue link through the server's link route
    let ans_link = call(
        &room,
        Some(&reviewer),
        Method::POST,
        &format!("/api/operations/verification/documents/{doc_id}/link"),
        Some((json!({"expectedVersion": 1}), "IDEM-LINK-EXP-1")),
        None,
    )
    .await;
    let data_link = ans_link.ok("issue link for expired test");
    let server_url = data_link["url"]
        .as_str()
        .expect("link response carries url");
    let server_token = server_url
        .strip_prefix("/api/verification/content/")
        .unwrap_or(server_url);

    // 2. Decrypt token with PiiCipher and b"xc-verification-download-link-v1" to extract session and actor_id claims
    let keys = Arc::new(
        PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
            .expect("the test key file parses"),
    );
    let cipher = PiiCipher::new(keys.clone());
    let (key_id, hex_bytes) = server_token
        .split_once('.')
        .expect("server token is key_id.hex");
    let raw_bytes = hex_decode(hex_bytes).expect("valid hex bytes");
    let sealed = Sealed {
        key_id: key_id.to_string(),
        bytes: raw_bytes,
    };
    let decrypted = cipher
        .decrypt(b"xc-verification-download-link-v1", &sealed)
        .expect("decrypt server token");
    let claims: Value = serde_json::from_slice(&decrypted).expect("parse token claims json");
    let actor_id = claims["actor_id"].as_str().expect("actor_id in claims");
    let session = claims["session"].as_str().expect("session in claims");

    // 3. Truly expired token: re-sign with DownloadLink::sign at now - LINK_LIFETIME - 1 min
    let download_link = DownloadLink::new(keys);
    let expired_time = Utc::now() - LINK_LIFETIME - ChronoDuration::minutes(1);
    let signed = download_link.sign(doc_id, actor_id, session, expired_time);
    let expired_url = format!("/api/verification/content/{}", signed.token);

    let ans_expired = call(
        &room,
        Some(&reviewer),
        Method::GET,
        &expired_url,
        None,
        None,
    )
    .await;

    assert!(
        ans_expired.status == StatusCode::GONE || ans_expired.status == StatusCode::BAD_REQUEST,
        "expired link must be refused with 410 Gone or 400 Bad Request: {}",
        ans_expired.text
    );
    assert_eq!(
        ans_expired.code(),
        "linkExpired",
        "truly expired token must answer exactly linkExpired: {}",
        ans_expired.text
    );

    // 2. Garbage token: not signed by this server -> linkRefused
    let garbage_url = "/api/verification/content/expired-dummy-token-past-lifetime";
    let ans_garbage = call(&room, Some(&reviewer), Method::GET, garbage_url, None, None).await;

    assert!(
        ans_garbage.status == StatusCode::FORBIDDEN
            || ans_garbage.status == StatusCode::BAD_REQUEST,
        "garbage token must be refused: {}",
        ans_garbage.text
    );
    assert_eq!(
        ans_garbage.code(),
        "linkRefused",
        "garbage token must answer linkRefused: {}",
        ans_garbage.text
    );

    room.finish().await;
}

// =================================================================================================
// `verification_queue` reads the customers' identities in one query and takes the last 4 *characters*
// (BE controller claude/7ca80c; up-front tests by claude/d7aa64). Today
// `api/src/presentation/http/v3/views/verification_queue.rs:145-190` runs one `customer_identities` query per
// queue row, and `s[s.len() - 4..]` slices bytes, which panics inside a multi-byte character.
// =================================================================================================

/// Seeds an identity for `customer_id` (PASSPORT, sealed like `seed_defaults` does), `age_minutes` old.
async fn seed_identity(
    room: &Room,
    customer_id: &str,
    number: &str,
    age_minutes: i64,
    status: &str,
) {
    common::install_test_pii_keys();
    let keys = Arc::new(
        PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
            .expect("the test PII key parses"),
    );
    let cipher = PiiCipher::new(keys.clone());
    let hasher = LookupHasher::new(&keys);
    let aad = format!("{customer_id}PASSPORT");
    let payload = serde_json::json!({ "number": number, "nationality": "THA" });
    let sealed = cipher.encrypt(aad.as_bytes(), payload.to_string().as_bytes());
    let lookup_hash = hasher.hash(IdentityType::Passport, number);
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, \
                                          nationality, verified_at, status, created_at) \
         VALUES (gen_random_uuid(), $1, $2, 'PASSPORT', $3, $4, $5, 'THA', now(), $6, now() - make_interval(mins => $7::int))",
    )
    .bind(Uuid::from_str(ORG_1).unwrap())
    .bind(customer_id)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(&lookup_hash[..])
    .bind(status)
    .bind(age_minutes as i32)
    .execute(&room.pool)
    .await
    .expect("seed a customer identity");
}

async fn queue_last4(room: &Room) -> Vec<(String, Value)> {
    let reviewer = Who::org_reviewer("11111111-1111-1111-1111-111111111111");
    let ans = call(
        room,
        Some(&reviewer),
        Method::GET,
        "/api/v3/views/verification_queue",
        None,
        None,
    )
    .await;
    let data = ans.ok("verification_queue");
    data["data"]
        .as_array()
        .unwrap_or_else(|| panic!("queue list: {}", ans.text))
        .iter()
        .map(|i| {
            (
                i["id"].as_str().unwrap_or("").to_owned(),
                i["identityLast4"].clone(),
            )
        })
        .collect()
}

async fn submitted(room: &Room, customer_id: &str, minutes_ago: i64) -> Uuid {
    let id = Uuid::new_v4();
    room.insert_case(
        id,
        ORG_1,
        customer_id,
        "SUBMITTED",
        Some(BRANCH_MAIN),
        Some(Utc::now() - ChronoDuration::minutes(minutes_ago)),
        None,
        None,
        None,
        None,
        1,
    )
    .await;
    id
}

#[actix_web::test]
async fn queue_rows_each_show_their_own_customers_last4() {
    let room = Room::start().await;
    // CX_1's passport ends 1234 (seed_defaults). CX_2 gets one ending 6789. CX_3 has none.
    seed_identity(&room, "CX_2", "BB5556789", 10, "ACTIVE").await;
    let c1 = submitted(&room, "CX_1", 30).await;
    let c2 = submitted(&room, "CX_2", 20).await;
    let c3 = submitted(&room, "CX_3", 10).await;

    let rows = queue_last4(&room).await;
    assert_eq!(
        rows,
        vec![
            (c1.to_string(), json!("1234")),
            (c2.to_string(), json!("6789")),
            (c3.to_string(), Value::Null),
        ],
        "each row carries its own customer's last 4, a customer without an identity carries none"
    );

    room.finish().await;
}

#[actix_web::test]
async fn queue_uses_the_newest_active_identity_and_ignores_revoked_ones() {
    let room = Room::start().await;
    seed_identity(&room, "CX_2", "XX0001111", 120, "ACTIVE").await;
    seed_identity(&room, "CX_2", "XX0002222", 60, "ACTIVE").await;
    seed_identity(&room, "CX_2", "XX0003333", 1, "REVOKED").await;
    let c2 = submitted(&room, "CX_2", 5).await;

    let rows = queue_last4(&room).await;
    assert_eq!(rows, vec![(c2.to_string(), json!("2222"))]);

    room.finish().await;
}

#[actix_web::test]
async fn a_multibyte_identity_number_answers_its_last_four_characters() {
    let room = Room::start().await;
    // "1ก34": 4 characters, 6 bytes. Slicing the last 4 *bytes* cuts inside "ก" and panics on main.
    seed_identity(&room, "CX_3", "1ก34", 10, "ACTIVE").await;
    let c3 = submitted(&room, "CX_3", 5).await;

    let rows = queue_last4(&room).await;
    assert_eq!(rows, vec![(c3.to_string(), json!("1ก34"))]);

    room.finish().await;
}
