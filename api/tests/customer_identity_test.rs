//! CI-4 (#651) Parts 2 and 3: one customer per company with an encrypted, hashed passport (design
//! `docs/design/blueprint/customer-identity-consent.md` K1, K2, K4, K8, §4.3, §9.1 rows "CI-4 Part 2/3", and the
//! decisions recorded on #651: alpha-3 nationality, DEMO shares LIVE, `list_live` company-wide, keys installed once
//! per process).
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Signatures assumed ([required] in #651 §7):
//! - `xc_customer::install_pii_keys(Arc<PiiKeys>) -> Result<(), xc_customer::PiiKeysConflict>`;
//! - `xc_customer::application::customer::service::CustomerService` with the old call shapes (`new(BranchTx)`,
//!   `reserve(&BranchId, ReserveCustomer, now) -> AppResult<Reservation>`, `list_live(&BranchId)`,
//!   `confirm_in(tx, &BranchId, customer_id, &CustomerDetails)`), plus
//!   `reveal(&self, actor: &Actor, branch: &BranchId, customer_id: &str, reason: &str, now) -> AppResult<String>`
//!   answering the ACTIVE passport's number;
//! - `INVALID_NATIONALITY` = "สัญชาติไม่ถูกต้อง", answered as `AppError::Domain`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_customer::domain::customer::value_objects::customer_details::CustomerDetails;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const OTHER_KEYS: &str = "test-2:ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
/// [required] the sentence of an invalid passport nationality (lane controller's alpha-3 ruling on #651).
const INVALID_NATIONALITY: &str = "สัญชาติไม่ถูกต้อง";
const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";
/// The desk's reserve sentences, unchanged from v2 (§9.1 row "CI-4 Part 2": "the six reserve sentences").
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
const BAD_DATE: &str = "วันที่ไม่ถูกต้อง";
const BAD_MODE: &str = "กรุณาเลือกประเภทรายการ";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 9, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn keys(text: &str) -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(text).expect("the test PII key parses"))
}

fn person(passport: &str, nationality: &str, mode: &str) -> ReserveCustomer {
    ReserveCustomer {
        name: "Somchai Jaidee".to_owned(),
        passport_no: passport.to_owned(),
        nationality: nationality.to_owned(),
        birth_date: "1990-01-02".to_owned(),
        passport_expiry: "2031-06-11".to_owned(),
        mode: mode.to_owned(),
    }
}

fn refusal<T: std::fmt::Debug>(result: Result<T, AppError>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected AppError::Domain(<sentence>), got {other:?}"),
    }
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
        let schema = unique("ci4id");
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
        xc_customer::install_pii_keys(keys(TEST_KEYS)).expect("one test PII key per process");
        for b in ["MAIN", "B2"] {
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(b)
            .execute(&pool)
            .await
            .expect("insert a branch");
        }
        Db { pool }
    }

    fn service(&self) -> CustomerService {
        CustomerService::new(BranchTx::new(Transactor::new(self.pool.clone())))
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn org(&self) -> String {
        sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
            .fetch_one(&self.pool)
            .await
            .expect("the organisation")
    }

    /// Non-bytea columns of the customer tables as JSON text, for plain-value checks.
    async fn stored_plain_text(&self) -> String {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT to_jsonb(c)::text FROM customers c \
             UNION ALL SELECT (to_jsonb(i) - 'encrypted_value' - 'lookup_hash')::text FROM customer_identities i",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read the customer tables");
        rows.join("\n")
    }

    /// The raw ciphertext bytes of the identity values.
    async fn stored_cipher_bytes(&self) -> Vec<Vec<u8>> {
        sqlx::query_scalar("SELECT encrypted_value FROM customer_identities")
            .fetch_all(&self.pool)
            .await
            .expect("read encrypted_value")
    }

    async fn actor(&self, role: Role) -> Actor {
        Actor::new(
            &uuid::Uuid::new_v4().to_string(),
            &self.org().await,
            vec![ScopedRole::new(
                role,
                Some(BranchId::new("MAIN")),
                at(-3600),
                None,
            )],
            None,
        )
    }
}

// ── keys ─────────────────────────────────────────────────────────────────────

#[test]
fn installing_the_same_keys_again_is_fine_and_other_keys_are_refused() {
    xc_customer::install_pii_keys(keys(TEST_KEYS)).expect("the first install");
    xc_customer::install_pii_keys(keys(TEST_KEYS)).expect("equal keys again: Ok");
    assert!(
        xc_customer::install_pii_keys(keys(OTHER_KEYS)).is_err(),
        "other keys: Err(PiiKeysConflict), never an overwrite"
    );
    xc_customer::install_pii_keys(keys(TEST_KEYS))
        .expect("the first keys are still the installed ones");
}

// ── one customer per company (K1, K4) ────────────────────────────────────────

#[tokio::test]
async fn the_same_passport_at_two_branches_is_one_customer() {
    let db = Db::start().await;
    let s = db.service();
    let main = s
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "LIVE"),
            at(0),
        )
        .await
        .expect("reserve at MAIN");
    let b2 = s
        .reserve(
            &BranchId::new("B2"),
            person("AA1234567", "THA", "LIVE"),
            at(1),
        )
        .await
        .expect("reserve at B2");
    assert_eq!(
        b2.customer_id, main.customer_id,
        "K1: the company's customer"
    );
    assert!(!main.existing && b2.existing);
    assert_eq!(db.count("SELECT count(*) FROM customers").await, 1);
}

#[tokio::test]
async fn a_demo_reserve_of_a_live_passport_answers_the_live_customer() {
    let db = Db::start().await;
    let s = db.service();
    let live = s
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "LIVE"),
            at(0),
        )
        .await
        .expect("LIVE");
    let demo = s
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "DEMO"),
            at(1),
        )
        .await
        .expect("DEMO");
    assert_eq!(
        demo.customer_id, live.customer_id,
        "owner 2026-10-09: DEMO shares LIVE"
    );
    assert!(demo.existing);
}

#[tokio::test]
async fn spacing_dashes_and_case_of_a_passport_find_the_same_customer() {
    let db = Db::start().await;
    let s = db.service();
    let a = s
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "LIVE"),
            at(0),
        )
        .await
        .expect("first");
    let b = s
        .reserve(
            &BranchId::new("MAIN"),
            person("aa 123-4567", "tha", "LIVE"),
            at(1),
        )
        .await
        .expect("the same passport written differently");
    assert_eq!(b.customer_id, a.customer_id);
}

#[tokio::test]
async fn concurrent_reserves_of_one_passport_make_one_customer() {
    let db = Db::start().await;
    let mut tasks = Vec::new();
    for n in 0..12 {
        let s = db.service();
        let branch = if n % 2 == 0 { "MAIN" } else { "B2" };
        tasks.push(tokio::spawn(async move {
            s.reserve(
                &BranchId::new(branch),
                person("AA7654321", "DEU", "LIVE"),
                at(n),
            )
            .await
            .expect("reserve")
            .customer_id
        }));
    }
    let mut ids = std::collections::BTreeSet::new();
    for t in tasks {
        ids.insert(t.await.expect("join"));
    }
    assert_eq!(ids.len(), 1, "one customer: {ids:?}");
    assert_eq!(db.count("SELECT count(*) FROM customers").await, 1);
    assert_eq!(
        db.count("SELECT count(*) FROM customer_identities").await,
        1
    );
}

// ── the nationality is the MRZ alpha-3 code ──────────────────────────────────

#[tokio::test]
async fn a_nationality_that_is_not_three_letters_is_refused_and_nothing_is_written() {
    let db = Db::start().await;
    let s = db.service();
    for bad in ["DE", "THAI"] {
        let got = s
            .reserve(
                &BranchId::new("MAIN"),
                person("AA1234567", bad, "LIVE"),
                at(0),
            )
            .await;
        assert_eq!(refusal(got), INVALID_NATIONALITY, "{bad}");
    }
    assert_eq!(db.count("SELECT count(*) FROM customers").await, 0);
    assert_eq!(
        db.count("SELECT count(*) FROM customer_identities").await,
        0
    );
}

// ── the save's re-check (K4) ─────────────────────────────────────────────────

#[tokio::test]
async fn confirm_in_accepts_the_customers_own_passport_and_refuses_another() {
    let db = Db::start().await;
    let reserved = db
        .service()
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "LIVE"),
            at(0),
        )
        .await
        .expect("reserve");
    let mut tx = db.pool.begin().await.expect("a transaction");
    let own = CustomerDetails::parse(&person("aa1234567", "THA", "LIVE")).expect("details");
    CustomerService::confirm_in(&mut tx, &BranchId::new("B2"), &reserved.customer_id, &own)
        .await
        .expect("the customer's own passport, at another branch of the company");
    let other = CustomerDetails::parse(&person("ZZ9999999", "THA", "LIVE")).expect("details");
    let refused = CustomerService::confirm_in(
        &mut tx,
        &BranchId::new("MAIN"),
        &reserved.customer_id,
        &other,
    )
    .await;
    assert_eq!(refusal(refused), NOT_THIS_PASSPORT);
}

// ── nothing identifying is stored or listed in plain (K2, K8) ────────────────

#[tokio::test]
async fn the_passport_is_stored_only_encrypted_and_hashed() {
    let db = Db::start().await;
    db.service()
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "LIVE"),
            at(0),
        )
        .await
        .expect("reserve");
    let stored = db.stored_plain_text().await;
    for plain in ["AA1234567", "1234567", "4567", "1990-01-02", "2031-06-11"] {
        assert!(
            !stored.contains(plain),
            "{plain} is not stored in plain: {stored}"
        );
    }
    let ciphers = db.stored_cipher_bytes().await;
    assert!(!ciphers.is_empty(), "expected stored cipher bytes");
    for cipher in &ciphers {
        for full_val in ["AA1234567", "1990-01-02", "2031-06-11"] {
            let needle = full_val.as_bytes();
            assert!(
                !cipher.windows(needle.len()).any(|w| w == needle),
                "{full_val} found inside encrypted_value bytes"
            );
        }
    }
    assert_eq!(
        db.count("SELECT count(*) FROM customer_identities WHERE length(lookup_hash) = 32 AND key_id <> ''")
            .await,
        1,
        "a 32-byte lookup hash and the key id that encrypted it"
    );
}

#[tokio::test]
async fn list_live_lists_every_live_customer_of_the_company_and_no_identity_value() {
    let db = Db::start().await;
    let s = db.service();
    let main = s
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "LIVE"),
            at(0),
        )
        .await
        .expect("MAIN");
    let b2 = s
        .reserve(
            &BranchId::new("B2"),
            person("BB7654321", "DEU", "LIVE"),
            at(1),
        )
        .await
        .expect("B2");
    s.reserve(
        &BranchId::new("MAIN"),
        person("CC1111111", "JPN", "DEMO"),
        at(2),
    )
    .await
    .expect("a DEMO customer");
    let listed = s.list_live(&BranchId::new("MAIN")).await.expect("list");
    let ids: Vec<&str> = listed.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![main.customer_id.as_str(), b2.customer_id.as_str()],
        "every LIVE customer of the company, in the order created (owner 2026-10-09)"
    );
    let printed = format!("{listed:?}");
    for plain in ["AA1234567", "BB7654321", "1990-01-02"] {
        assert!(
            !printed.contains(plain),
            "no identity value in a list: {printed}"
        );
    }
    assert_eq!(listed[1].nationality, "DEU", "the nationality is shown");
}

// ── reveal (K8) ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn reveal_needs_identity_document_read_and_writes_one_audit_row_without_the_value() {
    let db = Db::start().await;
    let s = db.service();
    let reserved = s
        .reserve(
            &BranchId::new("MAIN"),
            person("AA1234567", "THA", "LIVE"),
            at(0),
        )
        .await
        .expect("reserve");
    let marketing = db.actor(Role::Marketing).await;
    let refused = s
        .reveal(
            &marketing,
            &BranchId::new("MAIN"),
            &reserved.customer_id,
            "checking",
            at(1),
        )
        .await;
    assert!(
        refused.is_err(),
        "MARKETING never holds IdentityDocumentRead"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM audit_events").await,
        0,
        "a refusal writes no audit row"
    );

    let teller = db.actor(Role::Teller).await;
    let value = s
        .reveal(
            &teller,
            &BranchId::new("MAIN"),
            &reserved.customer_id,
            "the customer asked",
            at(2),
        )
        .await
        .expect("TELLER holds IdentityDocumentRead");
    assert_eq!(value, "AA1234567", "the ACTIVE passport's number");

    assert_eq!(
        db.count("SELECT count(*) FROM audit_events").await,
        1,
        "one audit row"
    );
    let row: String = sqlx::query_scalar("SELECT row_to_json(a)::text FROM audit_events a")
        .fetch_one(&db.pool)
        .await
        .expect("the audit row");
    let hash_hex: String =
        sqlx::query_scalar("SELECT encode(lookup_hash, 'hex') FROM customer_identities")
            .fetch_one(&db.pool)
            .await
            .expect("the hash");
    assert!(row.contains(&teller.user_id), "who: {row}");
    assert!(row.contains(&reserved.customer_id), "which customer: {row}");
    assert!(row.contains("the customer asked"), "why: {row}");
    for secret in ["AA1234567", "4567", hash_hex.as_str()] {
        assert!(
            !row.contains(secret),
            "never the value, its last 4 or its hash: {row}"
        );
    }
}

// ── the desk's sentences (unchanged) ─────────────────────────────────────────

#[tokio::test]
async fn the_desks_reserve_sentences_are_unchanged() {
    let db = Db::start().await;
    let s = db.service();
    let main = BranchId::new("MAIN");
    assert_eq!(
        refusal(
            s.reserve(
                &BranchId::new("NOWHERE"),
                person("AA1234567", "THA", "LIVE"),
                at(0)
            )
            .await
        ),
        NO_SUCH_BRANCH
    );
    let mut nameless = person("AA1234567", "THA", "LIVE");
    nameless.name = String::new();
    assert_eq!(refusal(s.reserve(&main, nameless, at(0)).await), BAD_TEXT);
    let mut bad_date = person("AA1234567", "THA", "LIVE");
    bad_date.birth_date = "2026-02-30".to_owned();
    assert_eq!(refusal(s.reserve(&main, bad_date, at(0)).await), BAD_DATE);
    let mut bad_mode = person("AA1234567", "THA", "LIVE");
    bad_mode.mode = "TEST".to_owned();
    assert_eq!(refusal(s.reserve(&main, bad_mode, at(0)).await), BAD_MODE);
    assert_eq!(db.count("SELECT count(*) FROM customers").await, 0);
}
