//! VF-5 #1393 — who may view a submitted identity document, and how: `DocumentAccess` (a link for the case's reviewer
//! in scope or its own customer, one audit row per link, serving one link's bytes) and `DownloadLink` (a 60-second,
//! session-bound, server-signed token). Design `docs/design/blueprint/verification.md` V6, V8, V10, V17, §5, §12 S-3;
//! §9.1 row VF-5; ticket #1393 §4 item 1, §4 item 2 and §9 edge cases.
//! Written before the implementation by the test writer (claude/dell-claude-1, on the BE lane controller
//! claude/ad05bf8d's order, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The database comes from every migration in `api/migrations/` (VF-1's M1 `<version>_verification.sql` included).
//! Cases and documents are seeded as rows (VF-3's `CaseService` is another ticket's), with their bytes in an in-memory
//! `DocumentStore` (VF-2's trait). Each test works in a schema of its own. Needs `TEST_DATABASE_URL` naming an
//! existing database whose name ends in `_test`; nothing here skips.
//!
//! The clock is injected: every call takes `now`, so the 59 s / 61 s edges need no sleeping.
//!
//! Not pinned here (the tests accept either answer): the refusal for a removed or purged document beyond "no link"
//! and "no content" (#1393 §8 `[proposed: not found]`); whether a refused request writes an audit row; whether an
//! organisation-wide reviewer may open an at-branch case's document, and whether a branch reviewer may open an app
//! case's; the exact text of `Content-Disposition` after `attachment`; how the token is keyed from `PiiKeys`.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, Utc};
use futures::future::BoxFuture;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;
use xc_verification::application::access::document_access::{
    AccessError, CONTENT_PATH, DocumentAccess, IssuedLink,
};
use xc_verification::application::access::download_link::{DownloadLink, LINK_LIFETIME, LinkError};

/// The organisation OR-1's migration creates.
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
/// The PII key file the tests sign with (one 32-byte key).
const KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// Another server's key file: its tokens are not ours.
const OTHER_KEYS: &str = "other-1:ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
/// A document number printed inside every stored test image; no answer, error or audit row may carry it.
const DOCUMENT_NUMBER: &str = "AA1234567";
/// A text inside every stored test image; no answer, error or audit row may carry it (V6).
const MARKER: &str = "VF5-SECRET-BYTES-41d2";
const SESSION: &str = "session-a";
const OTHER_SESSION: &str = "session-b";
/// The seven staff roles V8 refuses, role by role (#1393 §4 item 1). BRANCH_MANAGER, TELLER and FINANCE hold
/// `IdentityDocumentRead` (`RoleGrants::seed` / §11.2) and are still refused.
const REFUSED_STAFF: [Role; 7] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
];
/// Every role name; none may appear as a literal in the access code (#1393 §4 item 2).
const ROLE_LITERALS: [&str; 18] = [
    "Role::",
    "EXECUTIVE",
    "ADMIN",
    "RATE_MANAGER",
    "BRANCH_MANAGER",
    "TELLER",
    "FINANCE",
    "MARKETING",
    "VERIFICATION_REVIEWER",
    "Executive",
    "Admin",
    "RateManager",
    "BranchManager",
    "Teller",
    "Finance",
    "Marketing",
    "VerificationReviewer",
    "CUSTOMER",
];
const MUST_FINISH: StdDuration = StdDuration::from_secs(60);

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

/// Whole seconds, so `expires_at` compares exactly.
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(Utc::now().timestamp(), 0).expect("a valid instant")
}

fn keys(text: &str) -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(text).expect("the test key file parses"))
}

fn link() -> DownloadLink {
    DownloadLink::new(keys(KEYS))
}

/// A PNG whose bytes carry the marker and a document number.
fn png() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(MARKER.as_bytes());
    b.extend_from_slice(DOCUMENT_NUMBER.as_bytes());
    b.extend_from_slice(unique("png").as_bytes());
    b
}

/// The token of an issued link: the URL is `CONTENT_PATH` + token.
fn token_of(link: &IssuedLink) -> String {
    link.url
        .strip_prefix(CONTENT_PATH)
        .unwrap_or_else(|| panic!("the url starts with {CONTENT_PATH}: {}", link.url))
        .to_owned()
}

/// `token` with one character in the middle changed to another URL-safe character.
fn tampered(token: &str) -> String {
    let mut chars: Vec<char> = token.chars().collect();
    assert!(chars.len() >= 8, "a token long enough to sign: {token}");
    let i = chars.len() / 2;
    chars[i] = if chars[i] == 'A' { 'B' } else { 'A' };
    chars.into_iter().collect()
}

fn header<'a>(headers: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

// ── actors ──────────────────────────────────────────────────────────────────

fn yesterday() -> DateTime<Utc> {
    Utc::now() - Duration::days(1)
}

fn staff(user: Uuid, org: &str, role: Role, branch: Option<&BranchId>) -> Actor {
    Actor::new(
        &user.to_string(),
        org,
        vec![ScopedRole::new(role, branch.cloned(), yesterday(), None)],
        None,
    )
}

fn customer_actor(user: Uuid, org: &str, customer_id: &str) -> Actor {
    Actor::new(
        &user.to_string(),
        org,
        vec![ScopedRole::new(Role::Customer, None, yesterday(), None)],
        Some(customer_id),
    )
}

// ── the in-memory DocumentStore ─────────────────────────────────────────────

#[derive(Default)]
struct MemStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
}

impl DocumentStore for MemStore {
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, Result<String, DocumentStoreError>> {
        Box::pin(async move {
            self.objects
                .lock()
                .expect("not poisoned")
                .insert(doc_ref.to_owned(), bytes.to_vec());
            Ok("test-1".to_owned())
        })
    }

    fn get<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<Vec<u8>, DocumentStoreError>> {
        Box::pin(async move {
            self.objects
                .lock()
                .expect("not poisoned")
                .get(doc_ref)
                .cloned()
                .ok_or_else(|| DocumentStoreError::NotFound(doc_ref.to_owned()))
        })
    }

    fn delete<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<(), DocumentStoreError>> {
        Box::pin(async move {
            self.objects.lock().expect("not poisoned").remove(doc_ref);
            Ok(())
        })
    }
}

// ── the database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    schema: String,
    store: Arc<MemStore>,
}

/// A customer with their user (`customers.user_id`).
struct Customer {
    id: String,
    user: Uuid,
}

/// A seeded case with one document.
struct Doc {
    id: Uuid,
    case_id: Uuid,
    doc_ref: String,
    bytes: Vec<u8>,
    owner: Customer,
}

impl Db {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The verification tests need a real PostgreSQL and do not skip; \
                 point it at an existing database whose name ends in `_test`"
            )
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("vf5");
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
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        {
            let _slot = STARTING.acquire().await.expect("never closed");
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
            Migrator::new(dir.as_path())
                .await
                .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
                .run(&pool)
                .await
                .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));
        }
        Self {
            pool,
            schema,
            store: Arc::new(MemStore::default()),
        }
    }

    async fn finish(self) {
        let Db { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    fn access(&self) -> DocumentAccess {
        DocumentAccess::new(
            self.pool.clone(),
            self.store.clone(),
            Arc::new(PgAuditLog::new(self.pool.clone())),
            link(),
        )
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

    async fn organisation(&self) -> String {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('{id}', '{}', 'Asia/Bangkok', now())",
            unique("org")
        ))
        .await;
        id.to_string()
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

    async fn customer(&self, org: &str) -> Customer {
        let user = self.user(org).await;
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'Somchai', 'LIVE', now(), '{user}')"
        ))
        .await;
        Customer { id, user }
    }

    async fn branch(&self, org: &str) -> BranchId {
        let id = unique("B").replace('_', "-");
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{id}', '{id}', '{org}'::uuid)"
        ))
        .await;
        BranchId::new(id)
    }

    /// A SUBMITTED case of a new customer of `org` (an app case when `branch` is `None`, V17), with one PNG document
    /// whose bytes are in the store.
    async fn document(&self, org: &str, branch: Option<&BranchId>) -> Doc {
        let owner = self.customer(org).await;
        let case_id = Uuid::new_v4();
        let branch_sql = branch.map_or("NULL".to_owned(), |b| format!("'{}'", b.as_str()));
        self.exec(&format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, status, policy_version, branch_id, \
                 submitted_at, created_at) \
             VALUES ('{case_id}', '{org}'::uuid, '{}', 'SUBMITTED', 1, {branch_sql}, now(), now())",
            owner.id
        ))
        .await;
        let id = Uuid::new_v4();
        let doc_ref = format!("verification/{case_id}/{id}");
        let bytes = png();
        self.store
            .put(&doc_ref, &bytes)
            .await
            .expect("store the document");
        sqlx::query(
            "INSERT INTO verification_documents (id, organization_id, case_id, document_type, encrypted_object_ref, \
                 key_id, checksum, mime, size_bytes, uploaded_at, uploaded_by, created_at) \
             VALUES ($1, $2::uuid, $3, 'PASSPORT', $4, 'test-1', $5, 'image/png', $6, now(), $7, now())",
        )
        .bind(id)
        .bind(org)
        .bind(case_id)
        .bind(&doc_ref)
        .bind(Sha256::digest(&bytes).to_vec())
        .bind(i32::try_from(bytes.len()).expect("a small file"))
        .bind(owner.user)
        .execute(&self.pool)
        .await
        .expect("insert the document");
        Doc {
            id,
            case_id,
            doc_ref,
            bytes,
            owner,
        }
    }

    /// An organisation-wide VERIFICATION_REVIEWER of `org`.
    async fn reviewer(&self, org: &str) -> Actor {
        let user = self.user(org).await;
        staff(user, org, Role::VerificationReviewer, None)
    }

    async fn audit_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM audit_events").await
    }

    /// Everything the audit rows hold, as one text.
    async fn audit_text(&self) -> String {
        let (text,): (Option<String>,) = sqlx::query_as(
            "SELECT string_agg(concat_ws(' | ', actor_id, actor_role, scope, action, object_ref, branch_id, \
             before_redacted::text, after_redacted::text), E'\\n') FROM audit_events",
        )
        .fetch_one(&self.pool)
        .await
        .expect("read audit_events");
        text.unwrap_or_default()
    }
}

/// Neither the marker, the document number nor the store's ref is in `text`.
fn holds_no_secret(text: &str, doc: &Doc, what: &str) {
    for secret in [MARKER, DOCUMENT_NUMBER, doc.doc_ref.as_str()] {
        assert!(
            !text.contains(secret),
            "{what} must not carry `{secret}`: {text}"
        );
    }
}

// ── DownloadLink: lifetime, session, signature (S-3, V10) ───────────────────

/// #1393 §9: valid at 59 s, expired at 61 s (injected clock); the lifetime is 60 s (S-3).
#[test]
fn a_token_lives_sixty_seconds_valid_at_59_expired_at_61() {
    let t0 = now();
    let document = Uuid::new_v4();
    let signed = link().sign(document, "user-1", SESSION, t0);
    assert_eq!(LINK_LIFETIME, Duration::seconds(60));
    assert_eq!(signed.expires_at, t0 + Duration::seconds(60));
    assert!(
        !signed
            .token
            .contains(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace()),
        "the token goes in a URL path: {}",
        signed.token
    );

    let claims = link()
        .verify(&signed.token, SESSION, t0 + Duration::seconds(59))
        .expect("valid at 59 s");
    assert_eq!(claims.document_id, document, "names one document");
    assert_eq!(claims.actor_id, "user-1", "names one actor");
    assert_eq!(claims.expires_at, signed.expires_at);

    assert_eq!(
        link().verify(&signed.token, SESSION, t0 + Duration::seconds(61)),
        Err(LinkError::Expired),
        "expired at 61 s"
    );
}

/// #1393 §9: a token copied to another session is refused (S-3 "ผูกกับ session").
#[test]
fn a_token_used_in_another_session_is_refused() {
    let t0 = now();
    let signed = link().sign(Uuid::new_v4(), "user-1", SESSION, t0);
    assert_eq!(
        link().verify(&signed.token, OTHER_SESSION, t0 + Duration::seconds(1)),
        Err(LinkError::OtherSession)
    );
    assert!(
        link()
            .verify(&signed.token, SESSION, t0 + Duration::seconds(1))
            .is_ok(),
        "its own session still opens it"
    );
}

/// #1393 §9: a tampered token (one character changed) is refused; so is a token another key signed (V10 "signed by
/// the server").
#[test]
fn a_tampered_token_or_one_signed_with_another_key_is_invalid() {
    let t0 = now();
    let signed = link().sign(Uuid::new_v4(), "user-1", SESSION, t0);
    let at = t0 + Duration::seconds(1);
    assert_eq!(
        link().verify(&tampered(&signed.token), SESSION, at),
        Err(LinkError::Invalid),
        "one character changed"
    );
    assert_eq!(
        link().verify(&signed.token[..signed.token.len() - 4], SESSION, at),
        Err(LinkError::Invalid),
        "truncated"
    );
    assert_eq!(
        link().verify("", SESSION, at),
        Err(LinkError::Invalid),
        "empty"
    );
    let foreign = DownloadLink::new(keys(OTHER_KEYS)).sign(Uuid::new_v4(), "user-1", SESSION, t0);
    assert_eq!(
        link().verify(&foreign.token, SESSION, at),
        Err(LinkError::Invalid),
        "another server's key"
    );
}

/// Two links for one document differ by actor and by session (each names one actor and one session).
#[test]
fn a_token_names_its_actor_and_session() {
    let t0 = now();
    let document = Uuid::new_v4();
    let a = link().sign(document, "user-1", SESSION, t0);
    let b = link().sign(document, "user-2", SESSION, t0);
    let c = link().sign(document, "user-1", OTHER_SESSION, t0);
    assert_ne!(a.token, b.token);
    assert_ne!(a.token, c.token);
    assert_eq!(
        link()
            .verify(&b.token, SESSION, t0)
            .expect("b is valid")
            .actor_id,
        "user-2"
    );
}

// ── DocumentAccess: who may get a link (V8) ─────────────────────────────────

/// #1393 §4: an organisation-wide VERIFICATION_REVIEWER gets `{url, expiresAt}` for an app case's document, and the
/// link serves the stored bytes in its own session.
#[tokio::test]
async fn the_reviewer_gets_a_link_that_serves_the_document() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let reviewer = db.reviewer(ORG_1).await;
    let t0 = now();

    let issued = db
        .access()
        .link_for_reviewer(&reviewer, SESSION, doc.id, t0)
        .await
        .expect("the reviewer gets a link");
    assert_eq!(issued.expires_at, t0 + Duration::seconds(60), "S-3");
    holds_no_secret(&issued.url, &doc, "the link");

    let content = db
        .access()
        .open(&token_of(&issued), SESSION, t0 + Duration::seconds(59))
        .await
        .expect("the link opens in its session");
    assert_eq!(content.bytes, doc.bytes, "the stored bytes");
    assert_eq!(content.mime, "image/png");
    db.finish().await;
}

/// #1393 §4: the case's own customer (`Actor.customer_id`) gets a link to their document.
#[tokio::test]
async fn the_cases_own_customer_gets_a_link_to_their_document() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let owner = customer_actor(doc.owner.user, ORG_1, &doc.owner.id);
    let t0 = now();

    let issued = db
        .access()
        .link_for_owner(&owner, SESSION, doc.id, t0)
        .await
        .expect("the owner gets a link");
    assert_eq!(issued.expires_at, t0 + Duration::seconds(60));
    let content = db
        .access()
        .open(&token_of(&issued), SESSION, t0 + Duration::seconds(1))
        .await
        .expect("the owner's link opens");
    assert_eq!(content.bytes, doc.bytes);
    db.finish().await;
}

/// #1393 §4 role by role (V8): under RBAC §11.2, Executive and BranchManager hold VerificationReview
/// (Documents: Full) and can open reviewer links. Other staff roles get `outOfScope` on both doors;
/// TELLER and FINANCE hold `IdentityDocumentRead` and are still refused. All staff get `outOfScope` on the owner door.
#[tokio::test]
async fn every_other_staff_role_is_out_of_scope_role_by_role() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let t0 = now();
    for role in REFUSED_STAFF {
        let actor = staff(db.user(ORG_1).await, ORG_1, role, None);
        let as_reviewer = db
            .access()
            .link_for_reviewer(&actor, SESSION, doc.id, t0)
            .await;
        if RoleGrants::seed(role).contains(&Permission::VerificationReview) {
            assert!(
                as_reviewer.is_ok(),
                "{role:?} holds VerificationReview under §11.2: {as_reviewer:?}"
            );
        } else {
            assert!(
                matches!(as_reviewer, Err(AccessError::OutOfScope)),
                "{role:?} on the reviewer door: {as_reviewer:?}"
            );
            assert_eq!(
                as_reviewer.expect_err("refused").code(),
                "outOfScope",
                "{role:?}"
            );
        }
        let as_owner = db
            .access()
            .link_for_owner(&actor, SESSION, doc.id, t0)
            .await;
        assert!(
            matches!(as_owner, Err(AccessError::OutOfScope)),
            "{role:?} on the owner door: {as_owner:?}"
        );
    }
    db.finish().await;
}

/// #1393 §4: another customer of the same organisation gets `outOfScope` on both doors.
#[tokio::test]
async fn another_customer_is_out_of_scope() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let other = db.customer(ORG_1).await;
    let actor = customer_actor(other.user, ORG_1, &other.id);
    let t0 = now();
    let as_owner = db
        .access()
        .link_for_owner(&actor, SESSION, doc.id, t0)
        .await;
    assert!(
        matches!(as_owner, Err(AccessError::OutOfScope)),
        "another customer: {as_owner:?}"
    );
    let as_reviewer = db
        .access()
        .link_for_reviewer(&actor, SESSION, doc.id, t0)
        .await;
    assert!(
        matches!(as_reviewer, Err(AccessError::OutOfScope)),
        "a customer holds no VerificationReview: {as_reviewer:?}"
    );
    let reviewer = db.reviewer(ORG_1).await;
    let reviewer_as_owner = db
        .access()
        .link_for_owner(&reviewer, SESSION, doc.id, t0)
        .await;
    assert!(
        matches!(reviewer_as_owner, Err(AccessError::OutOfScope)),
        "a reviewer is not the case's customer: {reviewer_as_owner:?}"
    );
    db.finish().await;
}

/// V8 "in the case's scope": a VERIFICATION_REVIEWER of another organisation gets `outOfScope`.
#[tokio::test]
async fn a_reviewer_of_another_organisation_is_out_of_scope() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let elsewhere = db.organisation().await;
    let reviewer = db.reviewer(&elsewhere).await;
    let refused = db
        .access()
        .link_for_reviewer(&reviewer, SESSION, doc.id, now())
        .await;
    assert!(
        matches!(refused, Err(AccessError::OutOfScope)),
        "{refused:?}"
    );
    db.finish().await;
}

/// #1393 §9 (V17): an at-branch case's document — its branch's reviewer gets a link; a reviewer of another branch
/// gets `outOfScope`.
#[tokio::test]
async fn a_reviewer_of_another_branch_is_out_of_scope() {
    let db = Db::start().await;
    let b1 = db.branch(ORG_1).await;
    let b2 = db.branch(ORG_1).await;
    let doc = db.document(ORG_1, Some(&b1)).await;
    let t0 = now();

    let ours = staff(
        db.user(ORG_1).await,
        ORG_1,
        Role::VerificationReviewer,
        Some(&b1),
    );
    db.access()
        .link_for_reviewer(&ours, SESSION, doc.id, t0)
        .await
        .expect("the case's branch reviewer gets a link");

    let theirs = staff(
        db.user(ORG_1).await,
        ORG_1,
        Role::VerificationReviewer,
        Some(&b2),
    );
    let refused = db
        .access()
        .link_for_reviewer(&theirs, SESSION, doc.id, t0)
        .await;
    assert!(
        matches!(refused, Err(AccessError::OutOfScope)),
        "another branch: {refused:?}"
    );
    db.finish().await;
}

/// #1393 §9: a document already removed or purged gets no link, on either door.
#[tokio::test]
async fn a_removed_or_purged_document_gets_no_link() {
    let db = Db::start().await;
    let reviewer = db.reviewer(ORG_1).await;
    let t0 = now();
    for column in ["removed_at", "purged_at"] {
        let doc = db.document(ORG_1, None).await;
        db.exec(&format!(
            "UPDATE verification_documents SET encrypted_object_ref = NULL, {column} = now() WHERE id = '{}'",
            doc.id
        ))
        .await;
        let as_reviewer = db
            .access()
            .link_for_reviewer(&reviewer, SESSION, doc.id, t0)
            .await;
        assert!(as_reviewer.is_err(), "{column}: {as_reviewer:?}");
        assert!(
            !matches!(as_reviewer, Err(AccessError::Internal(_))),
            "{column} is an answer, not a failure: {as_reviewer:?}"
        );
        let owner = customer_actor(doc.owner.user, ORG_1, &doc.owner.id);
        let as_owner = db
            .access()
            .link_for_owner(&owner, SESSION, doc.id, t0)
            .await;
        assert!(as_owner.is_err(), "{column}: {as_owner:?}");
        assert!(
            !matches!(as_owner, Err(AccessError::Internal(_))),
            "{column} is an answer, not a failure: {as_owner:?}"
        );
    }
    db.finish().await;
}

// ── DocumentAccess: using a link ────────────────────────────────────────────

/// #1393 §4: a link used after its lifetime answers `linkExpired`; at 59 s it still opens.
#[tokio::test]
async fn a_link_used_after_sixty_seconds_is_link_expired() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let reviewer = db.reviewer(ORG_1).await;
    let t0 = now();
    let token = token_of(
        &db.access()
            .link_for_reviewer(&reviewer, SESSION, doc.id, t0)
            .await
            .expect("a link"),
    );
    db.access()
        .open(&token, SESSION, t0 + Duration::seconds(59))
        .await
        .expect("open at 59 s");
    let late = db
        .access()
        .open(&token, SESSION, t0 + Duration::seconds(61))
        .await;
    assert!(matches!(late, Err(AccessError::LinkExpired)), "{late:?}");
    assert_eq!(late.expect_err("expired").code(), "linkExpired");
    db.finish().await;
}

/// #1393 §4, §9: a link used by another session is refused and serves nothing; so is a tampered one.
#[tokio::test]
async fn a_link_used_by_another_session_or_tampered_is_refused() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let owner = customer_actor(doc.owner.user, ORG_1, &doc.owner.id);
    let t0 = now();
    let token = token_of(
        &db.access()
            .link_for_owner(&owner, SESSION, doc.id, t0)
            .await
            .expect("a link"),
    );
    let at = t0 + Duration::seconds(1);
    let copied = db.access().open(&token, OTHER_SESSION, at).await;
    assert!(
        matches!(copied, Err(AccessError::LinkRefused)),
        "another session: {copied:?}"
    );
    let forged = db.access().open(&tampered(&token), SESSION, at).await;
    assert!(
        matches!(forged, Err(AccessError::LinkRefused)),
        "tampered: {forged:?}"
    );
    for refused in [copied, forged] {
        holds_no_secret(
            &format!("{:?} {}", refused, refused.as_ref().expect_err("refused")),
            &doc,
            "the refusal",
        );
    }
    db.finish().await;
}

/// #1393 §8: a document removed after its link was issued serves nothing.
#[tokio::test]
async fn a_link_to_a_document_removed_since_serves_nothing() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let reviewer = db.reviewer(ORG_1).await;
    let t0 = now();
    let token = token_of(
        &db.access()
            .link_for_reviewer(&reviewer, SESSION, doc.id, t0)
            .await
            .expect("a link"),
    );
    db.exec(&format!(
        "UPDATE verification_documents SET encrypted_object_ref = NULL, removed_at = now() WHERE id = '{}'",
        doc.id
    ))
    .await;
    let gone = db
        .access()
        .open(&token, SESSION, t0 + Duration::seconds(1))
        .await;
    assert!(gone.is_err(), "removed since: {gone:?}");
    assert!(
        !matches!(gone, Err(AccessError::Internal(_))),
        "an answer, not a failure: {gone:?}"
    );
    db.finish().await;
}

/// #1393 §4 (V10): the content answers `Cache-Control: private, no-store`, `X-Content-Type-Options: nosniff`,
/// `Content-Disposition: attachment`, with its own type; no header carries the document number or the store's ref.
#[tokio::test]
async fn the_content_answers_private_no_store_nosniff_attachment() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let reviewer = db.reviewer(ORG_1).await;
    let t0 = now();
    let token = token_of(
        &db.access()
            .link_for_reviewer(&reviewer, SESSION, doc.id, t0)
            .await
            .expect("a link"),
    );
    let content = db
        .access()
        .open(&token, SESSION, t0 + Duration::seconds(1))
        .await
        .expect("open");
    let headers = content.headers();
    assert_eq!(header(&headers, "Cache-Control"), Some("private, no-store"));
    assert_eq!(header(&headers, "X-Content-Type-Options"), Some("nosniff"));
    let disposition = header(&headers, "Content-Disposition").expect("a Content-Disposition");
    assert!(
        disposition.starts_with("attachment"),
        "attachment: {disposition}"
    );
    assert_eq!(header(&headers, "Content-Type"), Some("image/png"));
    for (name, value) in &headers {
        holds_no_secret(&format!("{name}: {value}"), &doc, "a header");
    }
    db.finish().await;
}

// ── DocumentAccess: the audit (V6) ──────────────────────────────────────────

/// #1393 §4 (V6): each link issued writes exactly one `audit_events` row naming the document or its case, holding no
/// link, no token, no bytes, no document number and no store ref.
#[tokio::test]
async fn each_link_writes_one_audit_row_with_no_link_and_no_bytes() {
    let db = Db::start().await;
    let doc = db.document(ORG_1, None).await;
    let reviewer = db.reviewer(ORG_1).await;
    let owner = customer_actor(doc.owner.user, ORG_1, &doc.owner.id);
    let t0 = now();

    let before = db.audit_rows().await;
    let first = db
        .access()
        .link_for_reviewer(&reviewer, SESSION, doc.id, t0)
        .await
        .expect("a reviewer link");
    assert_eq!(
        db.audit_rows().await,
        before + 1,
        "one row for the reviewer's link"
    );
    let second = db
        .access()
        .link_for_owner(&owner, OTHER_SESSION, doc.id, t0)
        .await
        .expect("an owner link");
    assert_eq!(
        db.audit_rows().await,
        before + 2,
        "one row for the owner's link"
    );

    let text = db.audit_text().await;
    assert!(
        text.contains(&doc.id.to_string()) || text.contains(&doc.case_id.to_string()),
        "the rows name the document or its case: {text}"
    );
    holds_no_secret(&text, &doc, "the audit rows");
    for issued in [&first, &second] {
        assert!(
            !text.contains(&token_of(issued)),
            "no token in the audit: {text}"
        );
        assert!(!text.contains(&issued.url), "no link in the audit: {text}");
    }
    db.finish().await;
}

// ── #1393 §4 item 2: no role literal ────────────────────────────────────────

/// The access decision is `VerificationReview` in scope or owner = `Actor.customer_id`, never a role name: the code
/// of `src/application/access/` (comments aside) names `Permission::VerificationReview` and `customer_id`, and no role.
#[test]
fn the_access_check_names_the_permission_and_no_role() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/access");
    let mut code = String::new();
    for entry in std::fs::read_dir(&dir).expect("read src/application/access") {
        let path = entry.expect("an entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).expect("read a source file");
            for line in text.lines() {
                let line = line.split("//").next().unwrap_or("");
                code.push_str(line);
                code.push('\n');
            }
        }
    }
    for literal in ROLE_LITERALS {
        assert!(
            !code.contains(literal),
            "no role literal decides access: found `{literal}`"
        );
    }
    assert!(
        code.contains("Permission::VerificationReview"),
        "the reviewer door asks for VerificationReview"
    );
    assert!(
        code.contains("customer_id"),
        "the owner door compares Actor.customer_id"
    );
}
