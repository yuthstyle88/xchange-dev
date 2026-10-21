//! CU-L1 (#1376): the system read of a customer's verified phone (membership-points design D-15, §9.3 row CU-L1).
//!
//! Written before the implementation by the ticket author (claude/29df2ec5). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Locks the lane controller's rulings on #1376 (claude/ad05bf8d, 2026-10-12):
//! - the phone is the normalised E.164 `String` (no new `E164Phone` type);
//! - the audit row is written in the same transaction as the read, through `AuditLog::append`;
//! - a read that answers `None` is audited too.
//!
//! Signatures assumed ([required] in #1376 §7, scaffolded in `system_phone.rs`):
//! - `CustomerService::has_verified_phone_in(tx: &mut Transaction<'_, Postgres>, organization_id: Uuid,
//!   customer_id: &str) -> AppResult<bool>` (an associated fn, like `exchange_customer_in`);
//! - `CustomerService::verified_phone_for_system(&self, organization_id: Uuid, customer_id: &str,
//!   purpose: SystemReadPurpose, now: DateTime<Utc>) -> AppResult<Option<String>>`;
//! - `xc_customer::application::customer::SystemReadPurpose` with exactly one variant `Loyalty` and `as_str()`.
//!
//! Seeded PHONE rows follow D-16: `PiiCipher` with AAD `customer_id ‖ "PHONE"` over the E.164 value,
//! `lookup_hash = LookupHasher(PHONE, normalize(PHONE, raw))`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_customer::application::customer::SystemReadPurpose;
use xc_customer::application::customer::service::CustomerService;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys};

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const XC_ORG: &str = "00000000-0000-0000-0000-000000000001";
/// [required] design D-15: the audit row's action and role.
const ACTION: &str = "identity.system_read";
const SYSTEM: &str = "SYSTEM";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn xc_org() -> Uuid {
    Uuid::parse_str(XC_ORG).unwrap()
}

fn test_keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"))
}

fn e164(raw: &str) -> String {
    xc_kernel::pii::normalize(IdentityType::Phone, raw, None).expect("a valid phone")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// How a seeded PHONE row is stored.
#[derive(Clone, Copy)]
enum Phone {
    ActiveVerified,
    ActiveUnverified,
    RevokedVerified,
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn start() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("cul1");
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
            .max_connections(8)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
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
        // One key set per process; every test of this binary installs the same one.
        let _ = xc_customer::install_pii_keys(test_keys());
        Db { pool }
    }

    fn service(&self) -> CustomerService {
        CustomerService::new(BranchTx::new(Transactor::new(self.pool.clone())))
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A second organisation, YY.
    async fn other_org(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, $2, NULL, 'Asia/Bangkok', now())",
        )
        .bind(id)
        .bind(unique("YY"))
        .execute(&self.pool)
        .await
        .expect("insert an organisation");
        id
    }

    /// A LIVE customer of `org` with an ACTIVE passport and no phone.
    async fn customer(&self, org: Uuid) -> String {
        let id = format!("CX-{:08}", {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            SEQ.fetch_add(1, Ordering::Relaxed)
        });
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2, $1, 'Somchai Jaidee', 'LIVE', $3)",
        )
        .bind(&id)
        .bind(org)
        .bind(at(-86_400))
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        let keys = test_keys();
        let normalized =
            xc_kernel::pii::normalize(IdentityType::Passport, &unique("AA"), Some("THA"))
                .expect("a passport");
        let sealed = PiiCipher::new(keys.clone())
            .encrypt(format!("{id}PASSPORT").as_bytes(), b"{\"number\":\"AA1\"}");
        sqlx::query(
            "INSERT INTO customer_identities \
               (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, nationality, status) \
             VALUES ($1, $2, $3, 'PASSPORT', $4, $5, $6, 'THA', 'ACTIVE')",
        )
        .bind(Uuid::new_v4())
        .bind(org)
        .bind(&id)
        .bind(sealed.bytes)
        .bind(sealed.key_id)
        .bind(
            LookupHasher::new(&keys)
                .hash(IdentityType::Passport, &normalized)
                .to_vec(),
        )
        .execute(&self.pool)
        .await
        .expect("insert a passport identity");
        id
    }

    /// A PHONE identity of `customer` stored as D-16 writes it; returns the identity id.
    async fn phone(&self, org: Uuid, customer: &str, raw: &str, how: Phone) -> Uuid {
        let keys = test_keys();
        let value = e164(raw);
        let sealed = PiiCipher::new(keys.clone())
            .encrypt(format!("{customer}PHONE").as_bytes(), value.as_bytes());
        self.phone_row(
            org,
            customer,
            sealed.bytes,
            &sealed.key_id,
            LookupHasher::new(&keys)
                .hash(IdentityType::Phone, &value)
                .to_vec(),
            how,
        )
        .await
    }

    async fn phone_row(
        &self,
        org: Uuid,
        customer: &str,
        encrypted_value: Vec<u8>,
        key_id: &str,
        lookup_hash: Vec<u8>,
        how: Phone,
    ) -> Uuid {
        let (status, verified_at) = match how {
            Phone::ActiveVerified => ("ACTIVE", Some(at(-3600))),
            Phone::ActiveUnverified => ("ACTIVE", None),
            Phone::RevokedVerified => ("REVOKED", Some(at(-7200))),
        };
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO customer_identities \
               (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, verified_at, status) \
             VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, $7, $8)",
        )
        .bind(id)
        .bind(org)
        .bind(customer)
        .bind(encrypted_value)
        .bind(key_id)
        .bind(lookup_hash)
        .bind(verified_at)
        .bind(status)
        .execute(&self.pool)
        .await
        .expect("insert a phone identity");
        id
    }

    async fn has(&self, org: Uuid, customer: &str) -> bool {
        let mut tx = self.pool.begin().await.expect("begin");
        let answer = CustomerService::has_verified_phone_in(&mut tx, org, customer)
            .await
            .expect("has_verified_phone_in answers");
        tx.rollback().await.expect("rollback");
        answer
    }

    async fn read(&self, org: Uuid, customer: &str) -> Option<String> {
        self.service()
            .verified_phone_for_system(org, customer, SystemReadPurpose::Loyalty, at(0))
            .await
            .expect("verified_phone_for_system answers")
    }

    /// Every `identity.system_read` row as (actor_id, actor_role, object_ref, reason, the row's stored columns
    /// except `id`, `at` and `object_ref` as JSON text).
    async fn system_reads(&self) -> Vec<(String, String, String, Option<String>, String)> {
        sqlx::query_as(
            "SELECT actor_id, actor_role, object_ref, after_redacted->>'reason', \
                    (to_jsonb(a) - 'id' - 'at' - 'object_ref')::text \
             FROM audit_events a WHERE action = $1 ORDER BY id",
        )
        .bind(ACTION)
        .fetch_all(&self.pool)
        .await
        .expect("read audit_events")
    }

    async fn audit_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM audit_events")
            .fetch_one(&self.pool)
            .await
            .expect("count audit_events")
    }
}

/// The one audit row a `verified_phone_for_system` call must leave: role SYSTEM, `system:<purpose>`, the purpose
/// as the reason.
fn assert_one_system_read(rows: &[(String, String, String, Option<String>, String)]) {
    assert_eq!(
        rows.len(),
        1,
        "exactly one identity.system_read audit row per call, got {rows:?}"
    );
    let (actor_id, actor_role, _, reason, _) = &rows[0];
    let purpose = SystemReadPurpose::Loyalty.as_str();
    assert!(
        purpose.eq_ignore_ascii_case("loyalty"),
        "SystemReadPurpose::Loyalty is recorded as loyalty, got {purpose:?}"
    );
    assert_eq!(actor_role, SYSTEM, "actor_role");
    assert_eq!(
        actor_id,
        &format!("system:{purpose}"),
        "actor_id = system:<purpose>"
    );
    assert_eq!(
        reason.as_deref(),
        Some(purpose),
        "the reason is the purpose"
    );
}

// ── has_verified_phone_in ────────────────────────────────────────────────────

#[tokio::test]
async fn has_verified_phone_is_true_for_an_active_verified_phone() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveVerified)
        .await;

    assert!(db.has(xc_org(), &c).await);
}

#[tokio::test]
async fn has_verified_phone_is_false_without_a_phone_identity() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;

    assert!(
        !db.has(xc_org(), &c).await,
        "a passport alone is not a verified phone"
    );
    assert!(
        !db.has(xc_org(), "CX-NOBODY").await,
        "an unknown customer has no phone"
    );
}

#[tokio::test]
async fn has_verified_phone_is_false_for_a_revoked_phone() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::RevokedVerified)
        .await;

    assert!(!db.has(xc_org(), &c).await);
}

#[tokio::test]
async fn has_verified_phone_is_false_for_an_unverified_phone() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveUnverified)
        .await;

    assert!(!db.has(xc_org(), &c).await);
}

#[tokio::test]
async fn has_verified_phone_is_false_in_another_organisation() {
    let db = Db::start().await;
    let yy = db.other_org().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveVerified)
        .await;

    assert!(
        !db.has(yy, &c).await,
        "XC's customer is not readable as YY's"
    );
    assert!(db.has(xc_org(), &c).await);
}

/// D-5/D-15: the existence check never decrypts, so a row whose ciphertext cannot be opened still counts.
#[tokio::test]
async fn has_verified_phone_never_decrypts() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone_row(
        xc_org(),
        &c,
        vec![0u8; 40],
        "test-1",
        vec![7u8; 32],
        Phone::ActiveVerified,
    )
    .await;

    assert!(db.has(xc_org(), &c).await);
}

/// Controller's ruling on #1376 (claude/ad05bf8d): the existence check writes no audit row, for a true or a false
/// answer. The transaction is committed so a row written inside it would be seen.
#[tokio::test]
async fn has_verified_phone_writes_no_audit_row() {
    let db = Db::start().await;
    let with_phone = db.customer(xc_org()).await;
    db.phone(xc_org(), &with_phone, "081-234-5678", Phone::ActiveVerified)
        .await;
    let without_phone = db.customer(xc_org()).await;
    let before = db.audit_rows().await;

    for (customer, expected) in [(&with_phone, true), (&without_phone, false)] {
        let mut tx = db.pool.begin().await.expect("begin");
        let answer = CustomerService::has_verified_phone_in(&mut tx, xc_org(), customer).await;
        assert!(
            matches!(answer, Ok(a) if a == expected),
            "has_verified_phone_in({customer}) must answer Ok({expected}), got {answer:?}"
        );
        tx.commit().await.expect("commit");
    }

    assert_eq!(
        db.audit_rows().await,
        before,
        "has_verified_phone_in writes no audit_events row"
    );
}

// ── verified_phone_for_system: the value ─────────────────────────────────────

#[tokio::test]
async fn system_read_returns_the_e164_phone_and_writes_one_audit_row() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    let identity = db
        .phone(xc_org(), &c, "081-234-5678", Phone::ActiveVerified)
        .await;
    let before = db.audit_rows().await;

    assert_eq!(db.read(xc_org(), &c).await.as_deref(), Some("+66812345678"));

    let rows = db.system_reads().await;
    assert_one_system_read(&rows);
    assert_eq!(
        rows[0].2,
        identity.to_string(),
        "object_ref is the identity id"
    );
    assert_eq!(db.audit_rows().await, before + 1, "one audit row in all");
}

#[tokio::test]
async fn system_read_audit_row_holds_no_phone_hash_or_last_four() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveVerified)
        .await;
    let value = e164("081-234-5678");
    let hash = LookupHasher::new(&test_keys()).hash(IdentityType::Phone, &value);

    assert_eq!(db.read(xc_org(), &c).await, Some(value.clone()));

    let rows = db.system_reads().await;
    assert_one_system_read(&rows);
    let (_, _, object_ref, _, stored) = &rows[0];
    for needle in [value.as_str(), "0812345678", "812345678", "5678"] {
        assert!(
            !stored.contains(needle),
            "the audit row must not hold {needle:?}: {stored}"
        );
    }
    assert!(
        !stored.contains(&hex(&hash)) && !object_ref.contains(&hex(&hash)),
        "the audit row must not hold the lookup_hash: {stored}"
    );
    assert!(
        !object_ref.contains(&value) && !object_ref.contains("812345678"),
        "object_ref must not hold the phone: {object_ref}"
    );
}

#[tokio::test]
async fn system_read_returns_the_active_phone_not_a_revoked_one() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-111-1111", Phone::RevokedVerified)
        .await;
    let active = db
        .phone(xc_org(), &c, "+44 7700 900123", Phone::ActiveVerified)
        .await;

    assert_eq!(
        db.read(xc_org(), &c).await.as_deref(),
        Some("+447700900123")
    );
    let rows = db.system_reads().await;
    assert_one_system_read(&rows);
    assert_eq!(
        rows[0].2,
        active.to_string(),
        "object_ref is the ACTIVE identity"
    );
}

// ── verified_phone_for_system: None is audited too (controller's ruling) ─────

#[tokio::test]
async fn system_read_without_a_phone_is_none_and_still_audited() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;

    assert_eq!(db.read(xc_org(), &c).await, None);
    assert_one_system_read(&db.system_reads().await);
}

#[tokio::test]
async fn system_read_of_a_revoked_phone_is_none_and_still_audited() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::RevokedVerified)
        .await;

    assert_eq!(db.read(xc_org(), &c).await, None);
    assert_one_system_read(&db.system_reads().await);
}

#[tokio::test]
async fn system_read_of_an_unverified_phone_is_none_and_still_audited() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveUnverified)
        .await;

    assert_eq!(db.read(xc_org(), &c).await, None);
    assert_one_system_read(&db.system_reads().await);
}

#[tokio::test]
async fn system_read_in_another_organisation_is_none_and_still_audited() {
    let db = Db::start().await;
    let yy = db.other_org().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveVerified)
        .await;

    assert_eq!(
        db.read(yy, &c).await,
        None,
        "XC's customer is not readable as YY's"
    );
    assert_one_system_read(&db.system_reads().await);
}

// ── verified_phone_for_system: no value without its audit row ────────────────

/// §8: a decrypt failure is an internal error, never a silent `None`.
#[tokio::test]
async fn system_read_of_an_undecryptable_phone_is_an_error_not_none() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone_row(
        xc_org(),
        &c,
        vec![0u8; 40],
        "test-1",
        vec![7u8; 32],
        Phone::ActiveVerified,
    )
    .await;

    let answer = db
        .service()
        .verified_phone_for_system(xc_org(), &c, SystemReadPurpose::Loyalty, at(0))
        .await;
    assert!(answer.is_err(), "expected an error, got {answer:?}");
}

/// Controller's ruling: the audit row is part of the read. If the row cannot be written, the phone is not
/// answered.
#[tokio::test]
async fn system_read_whose_audit_row_is_refused_answers_no_phone() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveVerified)
        .await;
    db.exec(
        "CREATE FUNCTION refuse_audit() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'audit refused by the test'; END $$",
    )
    .await;
    db.exec(
        "CREATE TRIGGER refuse_audit BEFORE INSERT ON audit_events \
         FOR EACH ROW EXECUTE FUNCTION refuse_audit()",
    )
    .await;

    let answer = db
        .service()
        .verified_phone_for_system(xc_org(), &c, SystemReadPurpose::Loyalty, at(0))
        .await;
    assert!(
        answer.is_err(),
        "no phone without its audit row, got {answer:?}"
    );
    assert_eq!(db.audit_rows().await, 0);
}

/// Controller's ruling: the read and its audit row are one transaction. A commit that fails leaves no row and
/// answers no phone.
#[tokio::test]
async fn system_read_whose_transaction_fails_to_commit_answers_no_phone() {
    let db = Db::start().await;
    let c = db.customer(xc_org()).await;
    db.phone(xc_org(), &c, "081-234-5678", Phone::ActiveVerified)
        .await;
    db.exec(
        "CREATE FUNCTION refuse_commit() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'commit refused by the test'; END $$",
    )
    .await;
    db.exec(
        "CREATE CONSTRAINT TRIGGER refuse_commit AFTER INSERT ON audit_events \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION refuse_commit()",
    )
    .await;

    let answer = db
        .service()
        .verified_phone_for_system(xc_org(), &c, SystemReadPurpose::Loyalty, at(0))
        .await;
    assert!(
        answer.is_err(),
        "no phone when the audit row does not commit, got {answer:?}"
    );
    assert_eq!(db.audit_rows().await, 0, "the refused commit leaves no row");
}

// ── SystemReadPurpose ────────────────────────────────────────────────────────

/// D-15: exactly one variant, `Loyalty`, so no caller names an ad-hoc purpose. A second variant fails to compile
/// here.
#[test]
fn system_read_purpose_has_exactly_one_variant() {
    match SystemReadPurpose::Loyalty {
        SystemReadPurpose::Loyalty => {}
    }
}
