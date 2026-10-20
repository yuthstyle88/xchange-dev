//! CU-P1 (#1378): desk phone capture + OTP verification (migration M2, blueprint §6.3, §8, §9).
//!
//! Written before the implementation by the test-writer (gemini/pool-5, ordered by claude/ad05bf8d).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Stacks on CU-L1's test branch (`feat/cu-l1-tests-1376` @ 32f1d1e) which supplies
//! `CustomerService::has_verified_phone_in`.
//!
//! Locks the lane controller's rulings on #1378 (claude/ad05bf8d, 2026-10-12):
//! - (a) `code_hash = LookupHasher::hash(IdentityType::Phone, format!("xc:phone-otp:v1\u{1f}{challenge_id}\u{1f}{code}"))`
//!   using the existing public seam in `xc-kernel::pii::LookupHasher`;
//! - (b) wire error codes and statuses: validationFailed (400), rateLimited (429), codeExpired (409),
//!   tooManyAttempts (409), wrongCode (409), superseded (409), phoneInUse (409), smsUnavailable (503),
//!   RoleNotAllowed (400), outOfScope (403), notFound (404); NO 410;
//! - (c) SMS text: th, en, and zh templates differ, and each holds only the code and the acting branch's `branches.name`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor as LegacyActor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_customer::application::customer::phone_verification_service::{
    PhoneVerificationError, PhoneVerificationService,
};
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::phone_otp_policy::PhoneOtpPolicy;
use xc_customer::domain::phone_verification::PhoneVerification;
use xc_customer::domain::sms_sender::{BoxFuture, SmsSender};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys, Sealed};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::notify::NotifySmsSender;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn test_keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn xc_org() -> Uuid {
    Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("org 1")
}

const TELLER_USER_ID: &str = "00000000-0000-4000-8000-000000000010";
const FINANCE_USER_ID: &str = "00000000-0000-4000-8000-000000000020";

fn at(secs: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 4, 0, 0).unwrap() + Duration::seconds(secs)
}

// ── Test doubles ─────────────────────────────────────────────────────────────

struct FakeSmsSender {
    dispatched: Mutex<Vec<(String, String)>>,
    fail: AtomicBool,
}

impl FakeSmsSender {
    fn new() -> Self {
        Self {
            dispatched: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
        }
    }

    fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.dispatched.lock().unwrap().clone()
    }

    fn last_code(&self) -> Option<String> {
        let calls = self.dispatched.lock().unwrap();
        let (_, text) = calls.last()?;
        let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 6 {
            Some(digits[digits.len() - 6..].to_string())
        } else {
            None
        }
    }
}

impl SmsSender for FakeSmsSender {
    fn send<'a>(&'a self, phone: &'a str, text: &'a str) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) {
                return Err(AppError::Internal("Notify unavailable".to_owned()));
            }
            self.dispatched
                .lock()
                .unwrap()
                .push((phone.to_owned(), text.to_owned()));
            Ok(())
        })
    }
}

#[derive(sqlx::FromRow)]
struct AuditEventRow {
    id: i64,
    actor_id: Option<String>,
    actor_role: Option<String>,
    scope: Option<String>,
    branch_id: Option<String>,
    action: Option<String>,
    object_ref: Option<String>,
    before_redacted: Option<Value>,
    after_redacted: Option<Value>,
    at: DateTime<Utc>,
    organization_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct PhoneVerificationDbRow {
    id: Uuid,
    customer_id: String,
    branch_id: String,
    phone_encrypted: Vec<u8>,
    key_id: String,
    phone_lookup_hash: Vec<u8>,
    code_hash: Vec<u8>,
    #[allow(dead_code)]
    expires_at: DateTime<Utc>,
    attempts: i32,
    status: String,
    requested_by: Uuid,
    identity_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

// ── Test database harness ────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    #[allow(dead_code)]
    schema: String,
}

impl Db {
    async fn start() -> Self {
        let raw_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL must be set and name a database ending in `_test`.\n\
                 Never touch port 6381."
            )
        });
        assert!(
            !raw_url.contains(":6381"),
            "Safety: never touch port 6381 (live/shared service)"
        );
        let options = PgConnectOptions::from_str(&raw_url).expect("valid connection URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("cup1");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        // Runs migrations alone — table `phone_verifications` comes ONLY from migration M2.
        Migrator::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .as_path(),
        )
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");

        let _ = xc_customer::install_pii_keys(test_keys());
        for b in ["MAIN", "B2", "CHINATOWN"] {
            let name = match b {
                "CHINATOWN" => "Chinatown Branch",
                "B2" => "Siam Paragon",
                _ => "Main Branch",
            };
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET name = $2",
            )
            .bind(b)
            .bind(name)
            .execute(&pool)
            .await
            .expect("insert a branch");
        }

        let teller_user_id = Uuid::parse_str(TELLER_USER_ID).unwrap();
        let finance_user_id = Uuid::parse_str(FINANCE_USER_ID).unwrap();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $3, 'Route Teller', 'ACTIVE', 'test'), \
                    ($2, $3, 'Route Finance', 'ACTIVE', 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(teller_user_id)
        .bind(finance_user_id)
        .bind(xc_org())
        .execute(&pool)
        .await
        .expect("seed route users in xc_org");

        Db { pool, schema }
    }

    fn service(&self, sms_sender: Arc<dyn SmsSender>) -> PhoneVerificationService {
        PhoneVerificationService::new(
            BranchTx::new(Transactor::new(self.pool.clone())),
            sms_sender,
        )
    }

    async fn customer(&self, org: Uuid, lang: &str) -> String {
        let id = unique("CX");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, preferred_language, created_at) \
             VALUES ($1, $2, $1, 'Test Customer', 'LIVE', $3, now())",
        )
        .bind(&id)
        .bind(org)
        .bind(lang)
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        id
    }

    async fn other_org(&self) -> Uuid {
        let id = Uuid::new_v4();
        let code = unique("other");
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, $2, 'Other Org Co.', 'Asia/Bangkok', now())",
        )
        .bind(id)
        .bind(code)
        .execute(&self.pool)
        .await
        .expect("insert other org");
        id
    }

    async fn teller(&self, org: Uuid, branch: &str) -> (User, Uuid) {
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2, 'Test Teller', 'ACTIVE', 'test')",
        )
        .bind(user_id)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("insert a user");

        let actor = User::new(
            &user_id.to_string(),
            &org.to_string(),
            vec![ScopedRole::new(
                Role::Teller,
                Some(BranchId::new(branch)),
                at(-3600),
                None,
            )],
            None,
        );
        (actor, user_id)
    }

    /// Seed an existing ACTIVE verified PHONE identity for `customer` following D-16.
    async fn seed_verified_phone(&self, org: Uuid, customer_id: &str, e164: &str) -> Uuid {
        let keys = test_keys();
        let cipher = PiiCipher::new(keys.clone());
        let sealed = cipher.encrypt(format!("{customer_id}PHONE").as_bytes(), e164.as_bytes());
        let lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, e164);
        let identity_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO customer_identities ( \
                 id, customer_id, organization_id, type, encrypted_value, key_id, \
                 lookup_hash, verified_at, status, created_at, updated_at \
             ) VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, now(), 'ACTIVE', now(), now())",
        )
        .bind(identity_id)
        .bind(customer_id)
        .bind(org)
        .bind(&sealed.bytes)
        .bind(&sealed.key_id)
        .bind(&lookup_hash[..])
        .execute(&self.pool)
        .await
        .expect("seed customer PHONE identity");

        identity_id
    }

    async fn plain_text_in_audits(&self) -> String {
        let rows: Vec<AuditEventRow> = sqlx::query_as(
            "SELECT id, actor_id, actor_role, scope, branch_id, action, object_ref, \
                    before_redacted, after_redacted, at, organization_id \
             FROM audit_events ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .expect("fetch audit_events");

        rows.into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "actor_id": r.actor_id,
                    "actor_role": r.actor_role,
                    "scope": r.scope,
                    "branch_id": r.branch_id,
                    "action": r.action,
                    "object_ref": r.object_ref,
                    "before_redacted": r.before_redacted,
                    "after_redacted": r.after_redacted,
                    "at": r.at,
                    "organization_id": r.organization_id,
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn stored_plain_text(&self) -> String {
        let mut text = String::new();
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT concat_ws(' ', customer_no, display_name, mode, preferred_language) FROM customers",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read customers");
        for r in rows {
            text.push_str(&r.0);
            text.push('\n');
        }

        let pv_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_name = 'phone_verifications' \
             )",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(false);

        if pv_exists {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT concat_ws(' ', customer_id, branch_id, key_id, status) FROM phone_verifications",
            )
            .fetch_all(&self.pool)
            .await
            .expect("read phone_verifications");
            for r in rows {
                text.push_str(&r.0);
                text.push('\n');
            }
        }
        text
    }
}

// ── Gap 1: Migration M2 Tests ────────────────────────────────────────────────

#[tokio::test]
async fn test_phone_verifications_migration_file_exists() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with("_phone_verifications.sql") {
                found.push(name);
            }
        }
    }
    assert_eq!(
        found.len(),
        1,
        "exactly one migration named …_phone_verifications.sql (design §6.3, §8 M2): {found:?}"
    );
    let file_name = &found[0];
    let ts_part = file_name.split('_').next().unwrap_or("");
    let version: i64 = ts_part
        .parse()
        .expect("migration timestamp must be integer");
    assert!(
        version > 20261112110000,
        "M2 migration timestamp {version} must sort after main's newest (20261112110000)"
    );
}

#[tokio::test]
async fn test_phone_verifications_table_schema_and_indexes() {
    let db = Db::start().await;

    // Verify all 15 columns
    let cols: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'phone_verifications'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read columns from phone_verifications");

    assert!(
        !cols.is_empty(),
        "phone_verifications table must exist in test schema"
    );
    let expected_cols = [
        ("id", "uuid", "NO"),
        ("organization_id", "uuid", "NO"),
        ("customer_id", "text", "NO"),
        ("branch_id", "text", "NO"),
        ("phone_encrypted", "bytea", "NO"),
        ("key_id", "text", "NO"),
        ("phone_lookup_hash", "bytea", "NO"),
        ("code_hash", "bytea", "NO"),
        ("expires_at", "timestamp with time zone", "NO"),
        ("attempts", "integer", "NO"),
        ("status", "text", "NO"),
        ("requested_by", "uuid", "NO"),
        ("identity_id", "uuid", "YES"),
        ("created_at", "timestamp with time zone", "NO"),
        ("decided_at", "timestamp with time zone", "YES"),
    ];
    for (name, ty, nullable) in expected_cols {
        let c = cols
            .iter()
            .find(|c| c.0 == name)
            .unwrap_or_else(|| panic!("missing column phone_verifications.{name} in {cols:?}"));
        assert_eq!(c.1, ty, "phone_verifications.{name} data_type");
        assert_eq!(c.2, nullable, "phone_verifications.{name} is_nullable");
    }

    // Verify indexes: one_pending_per_customer and phone_verifications_rate
    let indexes: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT i.relname::text as index_name, \
                pg_get_indexdef(i.oid)::text as index_def, \
                ix.indisunique as is_unique \
         FROM pg_class t \
         JOIN pg_index ix ON t.oid = ix.indrelid \
         JOIN pg_class i ON i.oid = ix.indexrelid \
         JOIN pg_namespace n ON n.oid = t.relnamespace \
         WHERE n.nspname = current_schema() AND t.relname = 'phone_verifications'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read indexes for phone_verifications");

    let one_pending = indexes
        .iter()
        .find(|i| i.0 == "one_pending_per_customer")
        .expect("index one_pending_per_customer must exist");
    assert!(
        one_pending.2,
        "one_pending_per_customer index must be UNIQUE"
    );
    assert!(
        one_pending.1.contains("WHERE") && one_pending.1.contains("PENDING"),
        "one_pending_per_customer must have partial index condition WHERE status = 'PENDING': {}",
        one_pending.1
    );

    let rate_index = indexes
        .iter()
        .find(|i| i.0 == "phone_verifications_rate")
        .expect("index phone_verifications_rate must exist");
    assert!(
        rate_index.1.contains("organization_id")
            && rate_index.1.contains("phone_lookup_hash")
            && rate_index.1.contains("created_at"),
        "phone_verifications_rate must cover (organization_id, phone_lookup_hash, created_at): {}",
        rate_index.1
    );
}

#[tokio::test]
async fn test_phone_verifications_migration_down_drops_only_phone_verifications() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut m2_file = None;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with("_phone_verifications.sql") {
                m2_file = Some(entry.path());
                break;
            }
        }
    }
    let m2_path = m2_file.expect("M2 migration file must exist in api/migrations");
    let content = std::fs::read_to_string(&m2_path).expect("read M2 migration");
    let down_raw = content
        .split("-- down:")
        .nth(1)
        .or_else(|| content.split("-- Down:").nth(1))
        .or_else(|| content.split("-- down").nth(1))
        .expect("M2 migration must contain '-- down:' section");

    // Strip leading comment markers '-- ' so the repo's commented down section can be executed.
    // Filter only statements starting with recognized SQL keywords to avoid prose comments.
    let down_sql: String = down_raw
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let uncmt = trimmed.strip_prefix("--").map(str::trim).unwrap_or(trimmed);
            let first_word = uncmt
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_uppercase();
            match first_word.as_str() {
                "DROP" | "ALTER" | "DELETE" | "REVOKE" | "TRUNCATE" | "COMMIT" | "BEGIN" => {
                    Some(uncmt)
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        down_sql.contains("DROP TABLE") && down_sql.contains("phone_verifications"),
        "down SQL must contain DROP TABLE phone_verifications"
    );

    let db = Db::start().await;
    let org = xc_org();
    let cust = db.customer(org, "th").await;
    db.seed_verified_phone(org, &cust, "+66812345678").await;

    // Execute down section
    sqlx::raw_sql(AssertSqlSafe(down_sql))
        .execute(&db.pool)
        .await
        .expect("apply down SQL");

    // phone_verifications table must be dropped
    let pv_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'phone_verifications' \
         )",
    )
    .fetch_one(&db.pool)
    .await
    .expect("check phone_verifications dropped");
    assert!(!pv_exists, "M2 down must drop phone_verifications table");

    // customer_identities must remain intact with its rows
    let identities_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM customer_identities WHERE customer_id = $1")
            .bind(&cust)
            .fetch_one(&db.pool)
            .await
            .expect("check customer_identities rows");
    assert!(
        identities_count > 0,
        "M2 down must leave customer_identities rows intact"
    );
}

// ── Gap 2: Ruling (a) Code Hash Pinning & Domain Separation ──────────────────

#[tokio::test]
async fn test_code_hash_exact_formula_and_domain_separation() {
    let keys = test_keys();
    let _ = xc_customer::install_pii_keys(keys.clone());
    let challenge_id = Uuid::new_v4();
    let code = "123456";

    let expected_hash = LookupHasher::new(&keys).hash(
        IdentityType::Phone,
        &format!("xc:phone-otp:v1\u{1f}{challenge_id}\u{1f}{code}"),
    );
    let actual_hash = PhoneVerification::compute_code_hash(challenge_id, code);

    // Pin exact byte equality
    assert_eq!(
        actual_hash.as_slice(),
        expected_hash.as_slice(),
        "code_hash must byte-equal LookupHasher::hash(Phone, xc:phone-otp:v1\\u{{1f}}challenge\\u{{1f}}code)"
    );

    // Domain separation: OTP hash can never equal phone lookup_hash for the same raw code
    let phone_lookup = LookupHasher::new(&keys).hash(IdentityType::Phone, code);
    assert_ne!(
        actual_hash.as_slice(),
        phone_lookup.as_slice(),
        "code_hash must differ from lookup_hash for the same input (domain separation)"
    );

    // Mutant check 1: swapped input order goes RED
    let swapped_order = LookupHasher::new(&keys).hash(
        IdentityType::Phone,
        &format!("xc:phone-otp:v1\u{1f}{code}\u{1f}{challenge_id}"),
    );
    assert_ne!(
        actual_hash.as_slice(),
        swapped_order.as_slice(),
        "swapped input order mutant must be detected"
    );

    // Mutant check 2: constant hash goes RED
    assert_ne!(actual_hash.as_slice(), [7u8; 32].as_slice());

    // Sensitivity checks
    let diff_challenge = PhoneVerification::compute_code_hash(Uuid::new_v4(), code);
    assert_ne!(actual_hash.as_slice(), diff_challenge.as_slice());
    let diff_code = PhoneVerification::compute_code_hash(challenge_id, "654321");
    assert_ne!(actual_hash.as_slice(), diff_code.as_slice());
}

// ── §9 Rows 1 & 2: Start Thai and foreign numbers, read-back encryption ───────

#[tokio::test]
async fn test_start_thai_number_normalises_and_sends_sms() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, teller_user_id) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    let res = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect("start succeeds");

    assert_eq!(res.expires_at, at(300));
    assert_ne!(res.challenge_id, Uuid::nil());

    let calls = fake_sms.calls();
    assert_eq!(calls.len(), 1, "exactly one SMS sent");
    assert_eq!(calls[0].0, "+66812345678", "normalised E.164 phone");
    let code = fake_sms.last_code().expect("6-digit code in SMS");
    assert_eq!(code.len(), 6);

    // Read back challenge row directly from DB
    let keys = test_keys();
    let row: PhoneVerificationDbRow = sqlx::query_as(
        "SELECT id, customer_id, branch_id, phone_encrypted, key_id, phone_lookup_hash, \
                code_hash, expires_at, attempts, status, requested_by, identity_id, created_at \
         FROM phone_verifications WHERE id = $1",
    )
    .bind(res.challenge_id)
    .fetch_one(&db.pool)
    .await
    .expect("query phone_verifications row");

    assert_eq!(row.id, res.challenge_id);
    assert_eq!(row.customer_id, customer_id);
    assert_eq!(row.branch_id, "MAIN");
    assert_eq!(row.attempts, 0, "attempts start at 0");
    assert_eq!(row.status, "PENDING", "status is PENDING");
    assert_eq!(row.requested_by, teller_user_id);
    assert!(
        row.identity_id.is_none(),
        "identity_id is NULL while PENDING"
    );
    assert_eq!(row.created_at, at(0));

    // Pin code_hash byte equality per ruling (a)
    let expected_code_hash = LookupHasher::new(&keys).hash(
        IdentityType::Phone,
        &format!("xc:phone-otp:v1\u{1f}{}\u{1f}{code}", res.challenge_id),
    );
    assert_eq!(
        row.code_hash,
        expected_code_hash.to_vec(),
        "stored code_hash must byte-equal ruling (a) expected formula"
    );

    // Pin phone_lookup_hash
    let expected_lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, "+66812345678");
    assert_eq!(
        row.phone_lookup_hash,
        expected_lookup_hash.to_vec(),
        "stored phone_lookup_hash must byte-equal LookupHasher(Phone, +66...)"
    );

    // Decrypt phone_encrypted with AAD customer_id || "PHONE_OTP"
    let sealed = Sealed {
        key_id: row.key_id.clone(),
        bytes: row.phone_encrypted.clone(),
    };
    let decrypted = PiiCipher::new(keys.clone())
        .decrypt(format!("{customer_id}PHONE_OTP").as_bytes(), &sealed)
        .expect("decrypt phone_encrypted with challenge AAD");
    assert_eq!(
        String::from_utf8(decrypted).unwrap(),
        "+66812345678",
        "phone_encrypted holds E.164"
    );
}

#[tokio::test]
async fn test_start_foreign_number_starts_and_sends_sms() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "en").await;

    let res = service
        .start(&teller, &branch, &customer_id, "+14155552671", at(0))
        .await
        .expect("foreign start succeeds");

    assert_eq!(res.expires_at, at(300));
    let calls = fake_sms.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "+14155552671");
}

#[tokio::test]
async fn test_start_invalid_phone_number_fails_validation() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    for bad in ["not-a-phone", "123", "0000", "+9999999999999999999"] {
        let err = service
            .start(&teller, &branch, &customer_id, bad, at(0))
            .await
            .expect_err(&format!("bad phone '{bad}' must fail"));
        assert_eq!(err.code(), "validationFailed");
        assert_eq!(err.status_code(), 400);
    }

    assert!(fake_sms.calls().is_empty(), "no SMS sent for bad phone");
}

// ── §9 Rows 3–5: Wrong codes, locking, supersede, and expiry boundaries ───────

#[tokio::test]
async fn test_wrong_code_increments_attempts_and_fifth_locks() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    let start_res = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect("start succeeds");

    for attempt in 1..=4 {
        let err = service
            .confirm(
                &teller,
                &branch,
                &customer_id,
                start_res.challenge_id,
                "000000",
                at(attempt * 10),
            )
            .await
            .expect_err("wrong code must fail");
        assert_eq!(err.code(), "wrongCode");
        assert_eq!(err.status_code(), 409); // Ruling (b)

        let (attempts, status): (i32, String) =
            sqlx::query_as("SELECT attempts, status FROM phone_verifications WHERE id = $1")
                .bind(start_res.challenge_id)
                .fetch_one(&db.pool)
                .await
                .expect("query status");
        assert_eq!(attempts, attempt as i32);
        assert_eq!(status, "PENDING");
    }

    // 5th wrong code locks the challenge
    let err5 = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            start_res.challenge_id,
            "000000",
            at(50),
        )
        .await
        .expect_err("5th wrong code must lock");
    assert_eq!(err5.code(), "tooManyAttempts");
    assert_eq!(err5.status_code(), 409); // Ruling (b)

    let (attempts5, status5): (i32, String) =
        sqlx::query_as("SELECT attempts, status FROM phone_verifications WHERE id = $1")
            .bind(start_res.challenge_id)
            .fetch_one(&db.pool)
            .await
            .expect("query status");
    assert_eq!(attempts5, 5);
    assert_eq!(status5, "LOCKED");
}

#[tokio::test]
async fn test_right_code_on_locked_or_superseded_challenge_fails() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    // 1. Locked challenge
    let res1 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect("start succeeds");
    let code1 = fake_sms.last_code().expect("SMS code");

    for _ in 1..=5 {
        let _ = service
            .confirm(
                &teller,
                &branch,
                &customer_id,
                res1.challenge_id,
                "000000",
                at(10),
            )
            .await;
    }

    // Right code on LOCKED challenge fails
    let err_locked = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            res1.challenge_id,
            &code1,
            at(60),
        )
        .await
        .expect_err("right code on locked challenge must fail");
    assert_eq!(err_locked.code(), "tooManyAttempts");
    assert_eq!(err_locked.status_code(), 409);

    // 2. Superseded challenge
    let res2 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(100))
        .await
        .expect("second start succeeds");
    let code2 = fake_sms.last_code().expect("SMS code");

    let res3 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(120))
        .await
        .expect("third start succeeds");
    let code3 = fake_sms.last_code().expect("SMS code");

    // Right code on SUPERSEDED challenge res2 fails
    let err_superseded = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            res2.challenge_id,
            &code2,
            at(130),
        )
        .await
        .expect_err("right code on superseded challenge must fail");
    assert_eq!(err_superseded.code(), "superseded");
    assert_eq!(err_superseded.status_code(), 409);

    // Right code on active challenge res3 succeeds
    let confirm3 = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            res3.challenge_id,
            &code3,
            at(140),
        )
        .await
        .expect("confirm on newest challenge succeeds");
    assert!(confirm3.verified);
}

#[tokio::test]
async fn test_right_code_at_300s_succeeds_and_after_five_minutes_gives_code_expired() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    // Test 1: Exactly at boundary 300s -> SUCCEEDS
    let start_1 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect("start 1 succeeds");
    let code_1 = fake_sms.last_code().expect("SMS code");

    let confirm_boundary = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            start_1.challenge_id,
            &code_1,
            at(300),
        )
        .await
        .expect("confirm exactly at 300s boundary must succeed");
    assert!(confirm_boundary.verified);

    // Test 2: At 301s (after 5 minutes) -> codeExpired (409)
    let start_2 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(400))
        .await
        .expect("start 2 succeeds");
    let code_2 = fake_sms.last_code().expect("SMS code");

    let err_expired = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            start_2.challenge_id,
            &code_2,
            at(400 + 301),
        )
        .await
        .expect_err("confirm at 301s must give codeExpired");
    assert_eq!(err_expired.code(), "codeExpired");
    assert_eq!(err_expired.status_code(), 409); // Ruling (b): no 410

    // Status remains PENDING in DB (derived expiry, not stored)
    let status: String = sqlx::query_scalar("SELECT status FROM phone_verifications WHERE id = $1")
        .bind(start_2.challenge_id)
        .fetch_one(&db.pool)
        .await
        .expect("query status");
    assert_eq!(status, "PENDING", "EXPIRED is derived, not stored");
}

// ── §9 Rows 6–8: Phone in use, reverify transaction, and Notify down ──────────

#[tokio::test]
async fn test_phone_active_on_another_customer_gives_phone_in_use_and_writes_nothing() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;

    // Customer A has an ACTIVE verified phone
    let customer_a = db.customer(org, "th").await;
    let id_a = db
        .seed_verified_phone(org, &customer_a, "+66812345678")
        .await;

    // Customer B attempts to verify the SAME phone
    let customer_b = db.customer(org, "en").await;
    let start_res = service
        .start(&teller, &branch, &customer_b, "0812345678", at(0))
        .await
        .expect("start succeeds for B");
    let code = fake_sms.last_code().expect("SMS code");

    let err = service
        .confirm(
            &teller,
            &branch,
            &customer_b,
            start_res.challenge_id,
            &code,
            at(10),
        )
        .await
        .expect_err("must refuse phoneInUse");
    assert_eq!(err.code(), "phoneInUse");
    assert_eq!(err.status_code(), 409);

    // Customer B has no identity rows written
    let b_identities: i64 =
        sqlx::query_scalar("SELECT count(*) FROM customer_identities WHERE customer_id = $1")
            .bind(&customer_b)
            .fetch_one(&db.pool)
            .await
            .expect("count B identities");
    assert_eq!(b_identities, 0, "nothing written for customer B");

    // Customer A remains ACTIVE
    let a_status: String =
        sqlx::query_scalar("SELECT status FROM customer_identities WHERE id = $1")
            .bind(id_a)
            .fetch_one(&db.pool)
            .await
            .expect("status A");
    assert_eq!(a_status, "ACTIVE", "customer A identity remains ACTIVE");
}

#[tokio::test]
async fn test_reverify_same_customer_revokes_old_phone_and_activates_new_phone_in_same_tx() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    let old_id = db
        .seed_verified_phone(org, &customer_id, "+66811111111")
        .await;

    // Customer reverifies with a new phone
    let start_res = service
        .start(&teller, &branch, &customer_id, "0822222222", at(0))
        .await
        .expect("start new phone");
    let code = fake_sms.last_code().expect("SMS code");

    let confirm_res = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            start_res.challenge_id,
            &code,
            at(10),
        )
        .await
        .expect("confirm new phone succeeds");
    assert!(confirm_res.verified);
    let new_identity_id = confirm_res.identity_id;

    // Old identity must be REVOKED
    let old_status: String =
        sqlx::query_scalar("SELECT status FROM customer_identities WHERE id = $1")
            .bind(old_id)
            .fetch_one(&db.pool)
            .await
            .expect("query old status");
    assert_eq!(old_status, "REVOKED");

    // New identity must be ACTIVE, verified_at NOT NULL, D-16 encoding
    let (new_status, verified_at, cipher_text, key_id, lookup_hash): (
        String,
        Option<DateTime<Utc>>,
        Vec<u8>,
        String,
        Vec<u8>,
    ) = sqlx::query_as(
        "SELECT status, verified_at, encrypted_value, key_id, lookup_hash \
         FROM customer_identities WHERE id = $1",
    )
    .bind(new_identity_id)
    .fetch_one(&db.pool)
    .await
    .expect("query new identity");

    assert_eq!(new_status, "ACTIVE");
    assert!(verified_at.is_some());

    let keys = test_keys();
    let sealed = Sealed {
        key_id: key_id.clone(),
        bytes: cipher_text.clone(),
    };
    let decrypted = PiiCipher::new(keys.clone())
        .decrypt(format!("{customer_id}PHONE").as_bytes(), &sealed)
        .expect("decrypt new identity with D-16 AAD");
    assert_eq!(String::from_utf8(decrypted).unwrap(), "+66822222222");

    let expected_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, "+66822222222");
    assert_eq!(lookup_hash, expected_hash.to_vec());

    // CU-L1 has_verified_phone_in check
    let mut tx = db.pool.begin().await.expect("begin tx");
    let has_phone = CustomerService::has_verified_phone_in(&mut tx, org, &customer_id)
        .await
        .expect("has_verified_phone_in succeeds");
    tx.commit().await.expect("commit tx");
    assert!(has_phone);
}

#[tokio::test]
async fn test_notify_down_gives_sms_unavailable_and_writes_no_challenge_row() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    fake_sms.set_fail(true);
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    let err = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect_err("notify down must fail");
    assert_eq!(err.code(), "smsUnavailable");
    assert_eq!(err.status_code(), 503);

    // No challenge row committed
    let row_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM phone_verifications WHERE customer_id = $1")
            .bind(&customer_id)
            .fetch_one(&db.pool)
            .await
            .expect("count rows");
    assert_eq!(row_count, 0, "no challenge row written on SMS failure");
}

// ── Gap 3: Rate limits per customer, per phone, per teller & window expiry ────

#[tokio::test]
async fn test_rate_limit_per_customer_and_window_expiry() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    // 1. Per customer: ≤ 3 challenges / 15 minutes
    for i in 0..3 {
        service
            .start(&teller, &branch, &customer_id, "0812345678", at(i * 60))
            .await
            .expect("start within limit succeeds");
    }

    // 4th start at 14 minutes fails with rateLimited (429)
    let err4 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(14 * 60))
        .await
        .expect_err("4th start in 15m window must be rateLimited");
    assert_eq!(err4.code(), "rateLimited");
    assert_eq!(err4.status_code(), 429);

    // Outside the 15-minute window (at 16 minutes), start is allowed again
    let res_after_window = service
        .start(&teller, &branch, &customer_id, "0812345678", at(16 * 60))
        .await
        .expect("start after window expires must succeed");
    assert_ne!(res_after_window.challenge_id, Uuid::nil());
}

#[tokio::test]
async fn test_rate_limit_per_phone_and_window_expiry() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let target_phone = "0812345678";

    // 2. Per phone: ≤ 5 challenges / 1 hour across different customers
    for i in 0..5 {
        let cust = db.customer(org, "th").await;
        service
            .start(&teller, &branch, &cust, target_phone, at(i * 10 * 60))
            .await
            .expect("phone start within limit succeeds");
    }

    // 6th start for the same phone within 1 hour on a 6th customer fails
    let cust_6 = db.customer(org, "th").await;
    let err6 = service
        .start(&teller, &branch, &cust_6, target_phone, at(55 * 60))
        .await
        .expect_err("6th start for same phone in 1h window must be rateLimited");
    assert_eq!(err6.code(), "rateLimited");
    assert_eq!(err6.status_code(), 429);

    // Outside the 1-hour window (at 65 minutes), start for that phone is allowed again
    let res_after_phone_window = service
        .start(&teller, &branch, &cust_6, target_phone, at(65 * 60))
        .await
        .expect("phone start after 1h window must succeed");
    assert_ne!(res_after_phone_window.challenge_id, Uuid::nil());
}

#[tokio::test]
async fn test_rate_limit_per_teller_and_window_expiry() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;

    // 3. Per teller: ≤ 30 challenges / 1 hour
    for i in 0..30 {
        let cust = db.customer(org, "th").await;
        let phone = format!("081{:07}", i);
        service
            .start(&teller, &branch, &cust, &phone, at(i * 60))
            .await
            .expect("teller start within limit succeeds");
    }

    // 31st start by same teller within 1 hour fails
    let cust_31 = db.customer(org, "th").await;
    let err31 = service
        .start(&teller, &branch, &cust_31, "0899999999", at(55 * 60))
        .await
        .expect_err("31st start by same teller in 1h window must be rateLimited");
    assert_eq!(err31.code(), "rateLimited");
    assert_eq!(err31.status_code(), 429);

    // Outside the 1-hour window (at 65 minutes), start by that teller is allowed again
    let res_after_teller_window = service
        .start(&teller, &branch, &cust_31, "0899999999", at(65 * 60))
        .await
        .expect("teller start after 1h window must succeed");
    assert_ne!(res_after_teller_window.challenge_id, Uuid::nil());
}

// ── Gap 4: Wire Codes and Statuses Ruling (b) ─────────────────────────────────

#[tokio::test]
async fn test_wire_codes_and_statuses_match_ruling_b() {
    // Pin error code strings and exact status codes per ruling (b) table
    let errors: Vec<(PhoneVerificationError, &'static str, u16)> = vec![
        (
            PhoneVerificationError::ValidationFailed("bad".into()),
            "validationFailed",
            400,
        ),
        (PhoneVerificationError::RateLimited, "rateLimited", 429),
        (PhoneVerificationError::CodeExpired, "codeExpired", 409),
        (
            PhoneVerificationError::TooManyAttempts,
            "tooManyAttempts",
            409,
        ),
        (PhoneVerificationError::WrongCode, "wrongCode", 409),
        (PhoneVerificationError::Superseded, "superseded", 409),
        (PhoneVerificationError::PhoneInUse, "phoneInUse", 409),
        (
            PhoneVerificationError::SmsUnavailable,
            "smsUnavailable",
            503,
        ),
        (
            PhoneVerificationError::RoleNotAllowed,
            "roleNotAllowed",
            400,
        ),
        (PhoneVerificationError::OutOfScope, "outOfScope", 403),
        (PhoneVerificationError::NotFound, "notFound", 404),
    ];

    for (err, expected_code, expected_status) in errors {
        assert_eq!(err.code(), expected_code);
        assert_eq!(
            err.status_code(),
            expected_status,
            "error '{expected_code}' must have status {expected_status}"
        );
        // Explicitly assert that no error has status 410
        assert_ne!(
            err.status_code(),
            410,
            "v3 has no 410; '{expected_code}' must not map to 410"
        );
    }
}

// ── Gap 5: SMS text per language differs and holds only code and branch name ──

#[tokio::test]
async fn test_sms_text_differs_per_language_and_holds_only_code_and_branch_name() {
    let code = "839201";
    let shop = "Chinatown Branch";

    let th = PhoneOtpPolicy::sms_text("th", code, shop);
    let en = PhoneOtpPolicy::sms_text("en", code, shop);
    let zh = PhoneOtpPolicy::sms_text("zh", code, shop);

    // Each language template must differ
    assert_ne!(th, en, "th and en templates must differ");
    assert_ne!(en, zh, "en and zh templates must differ");
    assert_ne!(th, zh, "th and zh templates must differ");

    // Each template holds code and shop name
    for (lang, text) in [("th", &th), ("en", &en), ("zh", &zh)] {
        assert!(
            text.contains(code),
            "{lang} SMS text must contain code {code}: {text}"
        );
        assert!(
            text.contains(shop),
            "{lang} SMS text must contain shop name '{shop}': {text}"
        );
    }

    // DB integration: verify the acting branch's branches.name is injected and preferred_language template is used
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("CHINATOWN");
    let (teller, _) = db.teller(org, "CHINATOWN").await;

    // 1. "en" customer gets en template
    let customer_en = db.customer(org, "en").await;
    service
        .start(&teller, &branch, &customer_en, "0812345678", at(0))
        .await
        .expect("start succeeds for en");

    let calls = fake_sms.calls();
    assert_eq!(calls.len(), 1);
    let code_en = fake_sms.last_code().expect("SMS code for en");
    assert_eq!(
        calls[0].1,
        PhoneOtpPolicy::sms_text("en", &code_en, "Chinatown Branch"),
        "SMS text for en customer must match en template exactly"
    );
    // Must hold only code and shop name - never the plain phone or customer name
    assert!(
        !calls[0].1.contains("0812345678") && !calls[0].1.contains("+66812345678"),
        "SMS text must never contain the phone number: {}",
        calls[0].1
    );

    // 2. "zh" customer gets zh template
    let customer_zh = db.customer(org, "zh").await;
    service
        .start(&teller, &branch, &customer_zh, "0812345679", at(10))
        .await
        .expect("start succeeds for zh");

    let calls = fake_sms.calls();
    assert_eq!(calls.len(), 2);
    let code_zh = fake_sms.last_code().expect("SMS code for zh");
    assert_eq!(
        calls[1].1,
        PhoneOtpPolicy::sms_text("zh", &code_zh, "Chinatown Branch"),
        "SMS text for zh customer must match zh template exactly"
    );
    assert!(
        !calls[1].1.contains("0812345679") && !calls[1].1.contains("+66812345679"),
        "SMS text must never contain the phone number: {}",
        calls[1].1
    );

    // 3. "th" customer gets th template
    let customer_th = db.customer(org, "th").await;
    service
        .start(&teller, &branch, &customer_th, "0812345670", at(20))
        .await
        .expect("start succeeds for th");

    let calls = fake_sms.calls();
    assert_eq!(calls.len(), 3);
    let code_th = fake_sms.last_code().expect("SMS code for th");
    assert_eq!(
        calls[2].1,
        PhoneOtpPolicy::sms_text("th", &code_th, "Chinatown Branch"),
        "SMS text for th customer must match th template exactly"
    );
    assert!(
        !calls[2].1.contains("0812345670") && !calls[2].1.contains("+66812345670"),
        "SMS text must never contain the phone number: {}",
        calls[2].1
    );

    // All three language SMS texts must differ
    assert_ne!(calls[0].1, calls[1].1);
    assert_ne!(calls[1].1, calls[2].1);
    assert_ne!(calls[0].1, calls[2].1);
}

// ── Gap 6: Audit rows for start (sent/refused) and verify (verified/wrongCode/locked) ──

#[tokio::test]
async fn test_audit_rows_for_start_and_verify_outcomes_and_zero_secrets() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, teller_user_id) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    // 1. Audit on start success -> outcome: "sent"
    let start_res = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect("start succeeds");
    let code = fake_sms.last_code().expect("SMS code");

    // 2. Audit on start refused -> outcome: "refused"
    let _ = service
        .start(&teller, &branch, &customer_id, "invalid-phone", at(5))
        .await;

    // 3. Audit on wrong code -> outcome: "wrongCode"
    let wrong_code = "789012";
    let _ = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            start_res.challenge_id,
            wrong_code,
            at(10),
        )
        .await;

    // Repeat wrong codes until 5th attempt to trigger locked
    for attempt in 2..=4 {
        let _ = service
            .confirm(
                &teller,
                &branch,
                &customer_id,
                start_res.challenge_id,
                wrong_code,
                at(10 + attempt * 2),
            )
            .await;
    }

    // 4. Audit on 5th wrong code -> outcome: "locked"
    let _ = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            start_res.challenge_id,
            wrong_code,
            at(20),
        )
        .await;

    // Start a fresh challenge to test successful verify
    let start_2 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(30))
        .await
        .expect("start 2 succeeds");
    let code_2 = fake_sms.last_code().expect("SMS code 2");

    // 5. Audit on verify success -> outcome: "verified"
    let _ = service
        .confirm(
            &teller,
            &branch,
            &customer_id,
            start_2.challenge_id,
            &code_2,
            at(40),
        )
        .await
        .expect("confirm 2 succeeds");

    let audit_rows: Vec<AuditEventRow> = sqlx::query_as(
        "SELECT id, actor_id, actor_role, scope, branch_id, action, object_ref, \
                before_redacted, after_redacted, at, organization_id \
         FROM audit_events ORDER BY id ASC",
    )
    .fetch_all(&db.pool)
    .await
    .expect("fetch audit_events");

    assert_eq!(
        audit_rows.len(),
        9,
        "must record exactly 9 audit rows (one per event, no duplicates)"
    );

    let outcomes: Vec<String> = audit_rows
        .iter()
        .map(|r| {
            r.after_redacted
                .as_ref()
                .and_then(|v| v.get("outcome"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    r.before_redacted
                        .as_ref()
                        .and_then(|v| v.get("outcome"))
                        .and_then(|v| v.as_str())
                })
                .map(|s| s.to_string())
                .or_else(|| r.action.clone())
                .unwrap_or_default()
        })
        .collect();

    assert_eq!(
        outcomes,
        vec![
            "sent",
            "refused",
            "wrongCode",
            "wrongCode",
            "wrongCode",
            "wrongCode",
            "locked",
            "sent",
            "verified",
        ],
        "9 audit rows in exact sequence of outcomes"
    );

    let c1 = start_res.challenge_id.to_string();
    let c2 = start_2.challenge_id.to_string();

    let row_contains = |r: &AuditEventRow, needle: &str| -> bool {
        r.object_ref
            .as_deref()
            .map(|s| s.contains(needle))
            .unwrap_or(false)
            || r.after_redacted
                .as_ref()
                .map(|v| v.to_string().contains(needle))
                .unwrap_or(false)
            || r.before_redacted
                .as_ref()
                .map(|v| v.to_string().contains(needle))
                .unwrap_or(false)
            || r.action
                .as_deref()
                .map(|s| s.contains(needle))
                .unwrap_or(false)
    };

    // Rows 0, 2, 3, 4, 5, 6 belong to challenge 1 (sent, 4x wrongCode, locked)
    for idx in [0, 2, 3, 4, 5, 6] {
        assert!(
            row_contains(&audit_rows[idx], &c1),
            "audit row {idx} ({}) must contain challenge id {c1}",
            outcomes[idx]
        );
    }

    // Rows 7, 8 belong to challenge 2 (sent, verified)
    for idx in [7, 8] {
        assert!(
            row_contains(&audit_rows[idx], &c2),
            "audit row {idx} ({}) must contain challenge id {c2}",
            outcomes[idx]
        );
    }

    // Verify actor and customer are recorded in all rows
    let teller_str = teller_user_id.to_string();
    for (idx, r) in audit_rows.iter().enumerate() {
        assert!(
            r.actor_id.as_deref() == Some(&teller_str) || row_contains(r, &teller_str),
            "audit row {idx} ({}) must contain teller actor id",
            outcomes[idx]
        );
        assert!(
            row_contains(r, &customer_id),
            "audit row {idx} ({}) must contain customer id",
            outcomes[idx]
        );
    }

    let audits = db.plain_text_in_audits().await;

    // Verify secrets and hashes are NEVER in audit_events
    let keys = test_keys();
    let code_hash_hex = hex_encode(&PhoneVerification::compute_code_hash(
        start_res.challenge_id,
        &code,
    ));
    let lookup_hash_hex =
        hex_encode(&LookupHasher::new(&keys).hash(IdentityType::Phone, "+66812345678"));

    let db_plain = db.stored_plain_text().await;
    for secret in [
        "0812345678",
        "+66812345678",
        &code,
        &code_2,
        wrong_code,
        &code_hash_hex,
        &lookup_hash_hex,
    ] {
        assert!(
            !audits.contains(secret),
            "audit_events must never store secret or hash '{secret}': {audits}"
        );
        assert!(
            !db_plain.contains(secret),
            "db unencrypted text must never store secret or hash '{secret}': {db_plain}"
        );
    }
}

// ── Gap 7: Routes, Permissions (ExchangeTransact), and Notify Adapter ─────────

const HOST: &str = "localhost:8080";
const SAME_ORIGIN: &str = "http://localhost:8080";
const AS: &str = "x-test-as";

struct HeaderUsers;

fn user_uuid(spec: &str) -> Uuid {
    match spec {
        "TELLER@MAIN" | "TELLER" => Uuid::parse_str(TELLER_USER_ID).unwrap(),
        "FINANCE@MAIN" | "FINANCE" => Uuid::parse_str(FINANCE_USER_ID).unwrap(),
        "MARKETING@MAIN" | "MARKETING" => {
            Uuid::parse_str("00000000-0000-4000-8000-000000000030").unwrap()
        }
        "ADMIN@MAIN" | "ADMIN" => Uuid::parse_str("00000000-0000-4000-8000-000000000040").unwrap(),
        "BRANCH_MANAGER@MAIN" | "BRANCH_MANAGER" => {
            Uuid::parse_str("00000000-0000-4000-8000-000000000050").unwrap()
        }
        other => {
            if let Ok(u) = Uuid::parse_str(other) {
                u
            } else {
                Uuid::parse_str("00000000-0000-4000-8000-000000000099").unwrap()
            }
        }
    }
}

fn user_id(spec: &str) -> String {
    user_uuid(spec).to_string()
}

fn user_of(spec: &str) -> Option<User> {
    let (uid, role_str, branch_str) = if spec == TELLER_USER_ID {
        (TELLER_USER_ID.to_string(), "TELLER", "MAIN")
    } else if spec == FINANCE_USER_ID {
        (FINANCE_USER_ID.to_string(), "FINANCE", "MAIN")
    } else if let Some((id_part, rest)) = spec.split_once('|') {
        let (r, b) = rest.split_once('@')?;
        (id_part.to_string(), r, b)
    } else {
        let (r, b) = spec.split_once('@')?;
        (user_id(spec), r, b)
    };

    let role = match role_str {
        "FINANCE" => Role::Finance,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        "ADMIN" => Role::Admin,
        "BRANCH_MANAGER" => Role::BranchManager,
        _ => return None,
    };
    let branch = (branch_str != "*").then(|| BranchId::new(branch_str));
    Some(User::new(
        &uid,
        &xc_org().to_string(),
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn legacy_of(spec: &str) -> Sessions {
    let spec = spec.split_once('|').map(|(_, r)| r).unwrap_or(spec);
    let (uid, branch_str) = match spec {
        s if s == TELLER_USER_ID => (TELLER_USER_ID.to_string(), "MAIN"),
        s if s == FINANCE_USER_ID => (FINANCE_USER_ID.to_string(), "MAIN"),
        _ => match spec.split_once('@') {
            None => {
                return Sessions {
                    owner: None,
                    staff: None,
                };
            }
            Some((_, branch)) => (user_id(spec), branch),
        },
    };

    if branch_str == "*" {
        Sessions {
            owner: Some(LegacyActor::owner(uid)),
            staff: None,
        }
    } else {
        Sessions {
            owner: None,
            staff: LegacyActor::staff(uid, OldRole::Finance, BranchId::new(branch_str)),
        }
    }
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Box::pin(async move { legacy_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Box::pin(async move {
            let u = user_of(&spec);
            Callers {
                user: u.clone(),
                staff_user: u,
            }
        })
    }
}

#[actix_web::test]
async fn test_routes_customer_phone_start_and_confirm_with_permissions() {
    let db = Db::start().await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);

    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let org = xc_org();
    let cust_id = db.customer(org, "th").await;
    let challenge_id = Uuid::new_v4();

    // Seed real users rows with UUID ids in xc_org for the header callers
    let teller_user_id = Uuid::parse_str(TELLER_USER_ID).unwrap();
    let finance_user_id = Uuid::parse_str(FINANCE_USER_ID).unwrap();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $3, 'Route Teller', 'ACTIVE', 'test'), \
                ($2, $3, 'Route Finance', 'ACTIVE', 'test') \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(teller_user_id)
    .bind(finance_user_id)
    .bind(org)
    .execute(&db.pool)
    .await
    .expect("seed route users in xc_org");

    let teller_hdr = format!("{TELLER_USER_ID}|TELLER@MAIN");
    let finance_hdr = format!("{FINANCE_USER_ID}|FINANCE@MAIN");

    // 1. customerPhoneStart route: POST /api/customers/{id}/phone-verifications
    let start_uri = format!("/api/customers/{cust_id}/phone-verifications");
    let start_body = json!({
        "branch": "MAIN",
        "phone": "0812345678"
    });

    // Caller without ExchangeTransact (Role::Finance) is refused with roleNotAllowed (400)
    let req_refused = atest::TestRequest::post()
        .uri(&start_uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, finance_hdr.as_str()))
        .set_json(&start_body);
    let resp_refused = atest::call_service(&app, req_refused.to_request()).await;
    assert_eq!(resp_refused.status(), StatusCode::BAD_REQUEST);
    let body_refused: Value = atest::read_body_json(resp_refused).await;
    assert_eq!(
        body_refused["error"]["code"], "roleNotAllowed",
        "expected camelCase roleNotAllowed wire code"
    );

    // Caller with ExchangeTransact (Role::Teller) is authorized
    // Reaching the service with Notify unset gives 503 smsUnavailable (§9 "Notify unset")
    let req_authorized = atest::TestRequest::post()
        .uri(&start_uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, teller_hdr.as_str()))
        .set_json(&start_body);
    let resp_authorized = atest::call_service(&app, req_authorized.to_request()).await;
    assert_eq!(
        resp_authorized.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "authorized teller with unset Notify must receive 503 smsUnavailable from service"
    );
    let body_auth: Value = atest::read_body_json(resp_authorized).await;
    assert_eq!(
        body_auth["error"]["code"], "smsUnavailable",
        "authorized teller must receive smsUnavailable code from service"
    );

    // 2. customerPhoneConfirm route: POST /api/customers/{id}/phone-verifications/{challengeId}/confirm
    let confirm_uri =
        format!("/api/customers/{cust_id}/phone-verifications/{challenge_id}/confirm");
    let confirm_body = json!({
        "branch": "MAIN",
        "code": "123456"
    });

    // Caller without ExchangeTransact (Role::Finance) is refused with roleNotAllowed (400)
    let req_conf_refused = atest::TestRequest::post()
        .uri(&confirm_uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, finance_hdr.as_str()))
        .set_json(&confirm_body);
    let resp_conf_refused = atest::call_service(&app, req_conf_refused.to_request()).await;
    assert_eq!(resp_conf_refused.status(), StatusCode::BAD_REQUEST);
    let body_conf_refused: Value = atest::read_body_json(resp_conf_refused).await;
    assert_eq!(
        body_conf_refused["error"]["code"], "roleNotAllowed",
        "expected camelCase roleNotAllowed wire code"
    );

    // Caller with ExchangeTransact (Role::Teller) is authorized
    // Reaching the service with an unknown challenge id gives 404 notFound
    let unknown_challenge = Uuid::new_v4();
    let confirm_uri_unknown =
        format!("/api/customers/{cust_id}/phone-verifications/{unknown_challenge}/confirm");
    let req_conf_auth = atest::TestRequest::post()
        .uri(&confirm_uri_unknown)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, teller_hdr.as_str()))
        .set_json(&confirm_body);
    let resp_conf_auth = atest::call_service(&app, req_conf_auth.to_request()).await;
    assert_eq!(
        resp_conf_auth.status(),
        StatusCode::NOT_FOUND,
        "authorized teller with unknown challenge must receive 404 notFound from service"
    );
    let body_conf_auth: Value = atest::read_body_json(resp_conf_auth).await;
    assert_eq!(
        body_conf_auth["error"]["code"], "notFound",
        "authorized teller with unknown challenge must receive notFound code from service"
    );
}

#[tokio::test]
async fn test_notify_sms_sender_with_fake_notify_and_unset_base_url() {
    // 1. Unset NOTIFY_BASE_URL gives smsUnavailable
    let sender_unset = NotifySmsSender::new(None, None);
    let err_unset = sender_unset
        .send("+66812345678", "test code")
        .await
        .expect_err("unset NOTIFY_BASE_URL must fail");
    assert!(err_unset.to_string().contains("smsUnavailable"));

    // 2. Fake Notify on 127.0.0.1:0
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake Notify");
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        // Handle 2 requests: first 200 OK, second 500 Error
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut data = Vec::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = socket.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header_str = String::from_utf8_lossy(&data[..pos]);
                    let content_length = header_str
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            if k.trim().eq_ignore_ascii_case("content-length") {
                                v.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    if data.len() >= pos + 4 + content_length {
                        break;
                    }
                }
            }
            let req_str = String::from_utf8_lossy(&data);
            assert!(req_str.contains("POST /api/v1/notifications"));
            assert!(
                req_str.contains("x-api-key: secret-key")
                    || req_str.contains("X-API-Key: secret-key")
            );

            // Pin the Notify request body: channel "sms", E.164 phone, and text
            let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("");
            let json_body: Value = serde_json::from_str(body_part)
                .unwrap_or_else(|_| panic!("Notify request body must be valid JSON: {body_part}"));

            let channel = json_body
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert!(
                channel.eq_ignore_ascii_case("sms"),
                "channel must be 'sms', got: {channel} in {json_body}"
            );

            let phone = json_body
                .get("phone")
                .or_else(|| json_body.get("destination"))
                .or_else(|| json_body.get("to"))
                .or_else(|| json_body.get("recipient"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert_eq!(
                phone, "+66812345678",
                "Notify body must hold the E.164 phone: {json_body}"
            );

            let text = json_body
                .get("text")
                .or_else(|| json_body.get("message"))
                .or_else(|| json_body.get("content"))
                .or_else(|| json_body.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert_eq!(
                text, "Test code: 123456",
                "Notify body must hold the text: {json_body}"
            );

            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
            socket.write_all(response.as_bytes()).await.unwrap();
        }

        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let sender = NotifySmsSender::new(Some(base_url), Some("secret-key".to_owned()));

    // First call: 200 OK -> Ok(())
    let res_ok = sender.send("+66812345678", "Test code: 123456").await;
    assert!(res_ok.is_ok(), "200 response must succeed");

    // Second call: 500 -> Err("smsUnavailable")
    let res_err = sender.send("+66812345678", "Test code: 123456").await;
    assert!(
        res_err
            .expect_err("500 response must fail")
            .to_string()
            .contains("smsUnavailable")
    );
}

// ── Invariants: Supersede and Organization Scope ─────────────────────────────

#[tokio::test]
async fn test_start_supersedes_pending_challenge() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    let res1 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect("first start succeeds");

    let res2 = service
        .start(&teller, &branch, &customer_id, "0812345678", at(10))
        .await
        .expect("second start succeeds");

    assert_ne!(res1.challenge_id, res2.challenge_id);

    let status1: String =
        sqlx::query_scalar("SELECT status FROM phone_verifications WHERE id = $1")
            .bind(res1.challenge_id)
            .fetch_one(&db.pool)
            .await
            .expect("query status 1");
    assert_eq!(status1, "SUPERSEDED");

    let status2: String =
        sqlx::query_scalar("SELECT status FROM phone_verifications WHERE id = $1")
            .bind(res2.challenge_id)
            .fetch_one(&db.pool)
            .await
            .expect("query status 2");
    assert_eq!(status2, "PENDING");

    // Exactly one PENDING challenge per customer (enforced by unique partial index)
    let pending_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM phone_verifications WHERE customer_id = $1 AND status = 'PENDING'",
    )
    .bind(&customer_id)
    .fetch_one(&db.pool)
    .await
    .expect("count pending");
    assert_eq!(pending_count, 1);
}

#[tokio::test]
async fn test_customer_outside_organisation_gives_not_found() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms.clone());
    let org_a = xc_org();
    let org_b = db.other_org().await;
    let branch_a = BranchId::new("MAIN");
    let (teller_a, _) = db.teller(org_a, "MAIN").await;

    let customer_b = db.customer(org_b, "th").await;

    let err = service
        .start(&teller_a, &branch_a, &customer_b, "0812345678", at(0))
        .await
        .expect_err("customer outside org must fail with notFound 404 per ruling (b)");

    assert_eq!(
        err.code(),
        "notFound",
        "customer outside org must give notFound per ruling (b)"
    );
    assert_eq!(err.status_code(), 404);
    assert_eq!(
        fake_sms.calls().len(),
        0,
        "no SMS must be sent for another org's customer"
    );
    let pv_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'phone_verifications' \
         )",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap_or(false);
    if pv_exists {
        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM phone_verifications WHERE customer_id = $1")
                .bind(&customer_b)
                .fetch_one(&db.pool)
                .await
                .expect("count rows");
        assert_eq!(
            rows, 0,
            "no phone_verifications row must be written for another org's customer"
        );
    }
}

// ── Review rework concurrency tests (CU-P1, #1378) ───────────────────────────

#[tokio::test]
async fn test_concurrent_wrong_confirms_locks_challenge() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = Arc::new(db.service(fake_sms));
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    let start_res = service
        .start(&teller, &branch, &customer_id, "0812345678", at(0))
        .await
        .expect("start succeeds");

    // Spawn 6 concurrent wrong confirms
    let mut handles = Vec::new();
    for i in 0..6 {
        let svc = service.clone();
        let t = teller.clone();
        let b = branch.clone();
        let c_id = customer_id.clone();
        let ch_id = start_res.challenge_id;
        handles.push(tokio::spawn(async move {
            svc.confirm(&t, &b, &c_id, ch_id, "000000", at(10 + i))
                .await
        }));
    }

    let mut wrong_code_count = 0;
    let mut too_many_attempts_count = 0;
    for handle in handles {
        let res = handle.await.expect("join handle");
        match res {
            Err(PhoneVerificationError::WrongCode) => wrong_code_count += 1,
            Err(PhoneVerificationError::TooManyAttempts) => too_many_attempts_count += 1,
            other => panic!("unexpected confirm result: {other:?}"),
        }
    }

    assert_eq!(wrong_code_count, 4, "exactly 4 attempts produce wrongCode");
    assert_eq!(
        too_many_attempts_count, 2,
        "5th and 6th attempts produce tooManyAttempts"
    );

    let (attempts, status): (i32, String) =
        sqlx::query_as("SELECT attempts, status FROM phone_verifications WHERE id = $1")
            .bind(start_res.challenge_id)
            .fetch_one(&db.pool)
            .await
            .expect("query status");

    assert_eq!(attempts, 5);
    assert_eq!(status, "LOCKED");
}

#[tokio::test]
async fn test_concurrent_starts_respect_customer_rate_limit() {
    let db = Db::start().await;
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = Arc::new(db.service(fake_sms.clone()));
    let org = xc_org();
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;
    let customer_id = db.customer(org, "th").await;

    // Run 4 concurrent starts for the same customer
    let mut handles = Vec::new();
    for _ in 0..4 {
        let svc = service.clone();
        let t = teller.clone();
        let b = branch.clone();
        let c_id = customer_id.clone();
        handles.push(tokio::spawn(async move {
            svc.start(&t, &b, &c_id, "0812345678", at(0)).await
        }));
    }

    let mut successes = 0;
    let mut rate_limited = 0;
    for handle in handles {
        let res = handle.await.expect("join handle");
        match res {
            Ok(_) => successes += 1,
            Err(PhoneVerificationError::RateLimited) => rate_limited += 1,
            other => panic!("unexpected start result: {other:?}"),
        }
    }

    assert_eq!(successes, 3, "exactly 3 concurrent starts succeed");
    assert_eq!(
        rate_limited, 1,
        "4th start receives rateLimited coded refusal"
    );

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM phone_verifications WHERE customer_id = $1")
            .bind(&customer_id)
            .fetch_one(&db.pool)
            .await
            .expect("count challenges");
    assert_eq!(count, 3, "exactly 3 challenge rows persisted");

    assert_eq!(fake_sms.calls().len(), 3, "exactly 3 SMS sent");
}

#[actix_web::test]
async fn test_route_wrong_codes_commit_attempts_and_lock_challenge() {
    let db = Db::start().await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);

    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let org = xc_org();
    let cust_id = db.customer(org, "th").await;
    let branch = BranchId::new("MAIN");
    let (teller, _) = db.teller(org, "MAIN").await;

    // Seed real users row with TELLER_USER_ID for header caller
    let teller_user_id = Uuid::parse_str(TELLER_USER_ID).unwrap();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2, 'Route Teller', 'ACTIVE', 'test') \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(teller_user_id)
    .bind(org)
    .execute(&db.pool)
    .await
    .expect("seed route teller user in xc_org");

    let teller_hdr = format!("{TELLER_USER_ID}|TELLER@MAIN");

    // Start a challenge using service with fake SMS
    let fake_sms = Arc::new(FakeSmsSender::new());
    let service = db.service(fake_sms);
    let start_res = service
        .start(&teller, &branch, &cust_id, "0812345678", Utc::now())
        .await
        .expect("start succeeds");

    let confirm_uri = format!(
        "/api/customers/{cust_id}/phone-verifications/{}/confirm",
        start_res.challenge_id
    );

    // Send 6 wrong codes through the route
    let mut wrong_code_count = 0;
    let mut too_many_attempts_count = 0;

    for i in 1..=6 {
        let confirm_body = json!({
            "branch": "MAIN",
            "code": "000000"
        });
        let req = atest::TestRequest::post()
            .uri(&confirm_uri)
            .insert_header((header::HOST, HOST))
            .insert_header((header::ORIGIN, SAME_ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("idempotency-key", unique(&format!("k_{i}"))))
            .insert_header((AS, teller_hdr.as_str()))
            .set_json(&confirm_body);

        let resp = atest::call_service(&app, req.to_request()).await;
        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "attempt {i} must return 409 Conflict"
        );

        let body: Value = atest::read_body_json(resp).await;
        let err_code = body["error"]["code"].as_str().unwrap();

        if i <= 4 {
            assert_eq!(err_code, "wrongCode", "attempt {i} must return wrongCode");
            wrong_code_count += 1;
        } else if i == 5 {
            assert_eq!(
                err_code, "tooManyAttempts",
                "5th attempt locks and returns tooManyAttempts"
            );
            too_many_attempts_count += 1;
        } else {
            assert_eq!(
                err_code, "tooManyAttempts",
                "6th attempt on locked challenge returns tooManyAttempts"
            );
            too_many_attempts_count += 1;
        }
    }

    assert_eq!(wrong_code_count, 4);
    assert_eq!(too_many_attempts_count, 2);

    // The row must be LOCKED with attempts = 5
    let (attempts, status): (i32, String) =
        sqlx::query_as("SELECT attempts, status FROM phone_verifications WHERE id = $1")
            .bind(start_res.challenge_id)
            .fetch_one(&db.pool)
            .await
            .expect("query challenge status");

    assert_eq!(attempts, 5, "challenge row must have attempts = 5");
    assert_eq!(status, "LOCKED", "challenge row must be LOCKED");

    // Exactly 6 audit rows: 1 'sent' + 4 'wrongCode' + 1 'locked'
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT after_redacted->>'outcome' FROM audit_events \
         WHERE action = 'customer.phone_verification' \
         ORDER BY id ASC",
    )
    .fetch_all(&db.pool)
    .await
    .expect("fetch audit rows");

    let outcomes: Vec<&str> = audit_rows.iter().map(|(o,)| o.as_str()).collect();
    assert_eq!(
        outcomes,
        vec![
            "sent",
            "wrongCode",
            "wrongCode",
            "wrongCode",
            "wrongCode",
            "locked"
        ],
        "exactly 6 audit rows: 1 sent + 4 wrongCode + 1 locked"
    );
}
