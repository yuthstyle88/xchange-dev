//! Up-front tests: verifying a SAFE or SAFE_OPEN count compares it with the safe's ledger balance
//! (design `docs/design/blueprint/cash-shift-transfer.md` CA-7d: "A SAFE count fills `safe_variance`").
//!
//! Before the fix, `CashCountService::verify_in`'s branch for the other count kinds only marks the count
//! verified, and `submit_in` reads `expected` from a drawer only, so a SAFE count stores `expected` and
//! `difference` as NULL and no cash exception is written, however far the count is from the safe's ledger.
//!
//! Rows:
//! - S1 `safe_count_short_of_the_ledger_stores_expected_and_difference`,
//!   `safe_count_short_of_the_ledger_writes_one_safe_variance_exception`
//! - S2 `safe_open_count_short_of_the_ledger_stores_difference_and_safe_variance`
//! - S3 (guard, passes before and after the fix) `safe_count_matching_the_ledger_writes_no_exception`
//!
//! These tests pin only `expected`/`difference` and the exception row. Whether a SAFE difference passes the
//! count-difference FINANCE gate or posts SAFE ↔ VARIANCE is the ticket's ruling: the branch's limit is set
//! high and the USD the safe holds was funded at cost 0, so a gate valuing the difference clears it either way.
//!
//! Written by the test-writer before the implementation. Do not edit, delete or weaken.
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 24, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a valid decimal")
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn fresh() -> Db {
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
        let schema = unique("safe_count_variance");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        Db { pool }
    }

    /// A branch whose SAFE holds, through the ledger, USD 1,000 and nothing else, with a count-difference
    /// limit far above any difference these tests make.
    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&id)
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, 1000000, 'test', now())",
        )
        .bind(ORG)
        .bind(&id)
        .execute(&self.pool)
        .await
        .expect("set the branch's cash limit");
        fund_safe(&self.pool, &id, "USD", "1000.00").await;
        BranchId::new(id)
    }

    async fn person(&self, name: &str, roles: &[Role]) -> Actor {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let role_rows: Vec<RoleRow> = roles
            .iter()
            .map(|r| RoleRow {
                role_code: r.as_str(),
                branch_id: None,
                valid_from: from,
                valid_to: None,
            })
            .collect();
        let mut conn = self.pool.acquire().await.expect("acquire connection");
        let id = seed_person(&mut conn, ORG, name, &role_rows)
            .await
            .expect("seed person");
        Actor::new(
            &id,
            ORG,
            roles
                .iter()
                .map(|r| ScopedRole::new(*r, None, from, None))
                .collect(),
            None,
        )
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("denomination {currency} {face}: {e}"))
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    /// `(counted, expected, difference)` of one currency of a count.
    async fn total(
        &self,
        count: &str,
        currency: &str,
    ) -> (Decimal, Option<Decimal>, Option<Decimal>) {
        sqlx::query_as(
            "SELECT counted, expected, difference FROM cash_count_totals \
             WHERE count_id = $1::uuid AND currency = $2",
        )
        .bind(count)
        .bind(currency)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} total of count {count}: {e}"))
    }

    async fn cash_exceptions_for_count(&self, count_id: &str) -> Vec<serde_json::Value> {
        let rows: Vec<(serde_json::Value,)> =
            sqlx::query_as("SELECT data FROM cash_exceptions WHERE data->>'requestId' = $1")
                .bind(count_id)
                .fetch_all(&self.pool)
                .await
                .expect("read cash exceptions");
        rows.into_iter().map(|(d,)| d).collect()
    }
}

fn cmd(branch: &BranchId, kind: CountKind, lines: Vec<CountLine>) -> SubmitCount {
    SubmitCount {
        branch: branch.clone(),
        shift_id: None,
        kind,
        lines,
        note: None,
        idempotency_key: unique("count"),
    }
}

/// USD 100 × `notes`.
async fn usd_notes(db: &Db, notes: i32) -> Vec<CountLine> {
    vec![CountLine::new(
        "USD",
        db.denomination("USD", "100").await,
        notes,
    )]
}

/// Submits a `kind` count of the branch's safe by one BRANCH_MANAGER and verifies it by another; answers the
/// count's id. Each verify here is expected to succeed: the count-difference limit is far above the difference.
async fn count_the_safe(db: &Db, branch: &BranchId, kind: CountKind, notes: i32) -> String {
    let counter = db.person("Mana", &[Role::BranchManager]).await;
    let verifier = db.person("Krit", &[Role::BranchManager]).await;
    let counts = db.counts();
    let count = counts
        .submit(
            &counter,
            cmd(branch, kind, usd_notes(db, notes).await),
            at(0),
        )
        .await
        .unwrap_or_else(|e| panic!("submit the {} count: {e:?}", kind.as_str()));
    let verified = counts
        .verify(&verifier, &count.id, at(60))
        .await
        .unwrap_or_else(|e| panic!("verify the {} count: {e:?}", kind.as_str()));
    assert_eq!(
        verified.verified_by.as_deref(),
        Some(verifier.user_id.as_str()),
        "the {} count is verified by the verifier",
        kind.as_str()
    );
    count.id
}

fn assert_one_safe_variance_exception(exceptions: &[serde_json::Value], count_id: &str) {
    assert_eq!(
        exceptions.len(),
        1,
        "a count short of the safe's ledger writes exactly one cash exception: {exceptions:?}"
    );
    let exc = &exceptions[0];
    assert_eq!(exc["requestId"], count_id);
    assert_eq!(exc["status"], "OPEN");
    assert_eq!(exc["note"], format!("COUNT {count_id}"));
    assert_eq!(
        exc["safeVariance"]["USD"], "-100.00",
        "safeVariance holds the safe's USD difference as a decimal string: {exc}"
    );
    assert_eq!(
        exc["safeVariance"].as_object().map(|o| o.len()),
        Some(1),
        "safeVariance holds only the currency that differs: {exc}"
    );
    assert!(
        exc["cashVariance"].is_null(),
        "a safe count is no drawer count; cashVariance stays null: {exc}"
    );
    assert!(exc["targetDifference"].is_null(), "{exc}");
    assert_eq!(exc["at"], at(60).timestamp_millis());
}

// ── S1: a SAFE count short of the ledger ────────────────────────────────────

#[tokio::test]
async fn safe_count_short_of_the_ledger_stores_expected_and_difference() {
    let db = Db::fresh().await;
    let branch = db.branch("safe_short").await;

    // The safe's ledger holds USD 1,000; the count finds USD 900.
    let count_id = count_the_safe(&db, &branch, CountKind::Safe, 9).await;

    let (counted, expected, difference) = db.total(&count_id, "USD").await;
    assert_eq!(counted.normalize(), d("900"));
    assert_eq!(
        expected.map(|e| e.normalize()),
        Some(d("1000")),
        "a SAFE count's expected is the safe's ledger balance"
    );
    assert_eq!(
        difference.map(|x| x.normalize()),
        Some(d("-100")),
        "a SAFE count's difference is counted − the safe's ledger balance"
    );
}

#[tokio::test]
async fn safe_count_short_of_the_ledger_writes_one_safe_variance_exception() {
    let db = Db::fresh().await;
    let branch = db.branch("safe_short_exc").await;

    let count_id = count_the_safe(&db, &branch, CountKind::Safe, 9).await;

    let exceptions = db.cash_exceptions_for_count(&count_id).await;
    assert_one_safe_variance_exception(&exceptions, &count_id);
}

// ── S2: SAFE_OPEN behaves like SAFE ─────────────────────────────────────────

#[tokio::test]
async fn safe_open_count_short_of_the_ledger_stores_difference_and_safe_variance() {
    let db = Db::fresh().await;
    let branch = db.branch("safe_open_short").await;

    let count_id = count_the_safe(&db, &branch, CountKind::SafeOpen, 9).await;

    let (_, expected, difference) = db.total(&count_id, "USD").await;
    assert_eq!(
        expected.map(|e| e.normalize()),
        Some(d("1000")),
        "a SAFE_OPEN count's expected is the safe's ledger balance"
    );
    assert_eq!(
        difference.map(|x| x.normalize()),
        Some(d("-100")),
        "a SAFE_OPEN count's difference is counted − the safe's ledger balance"
    );

    let exceptions = db.cash_exceptions_for_count(&count_id).await;
    assert_one_safe_variance_exception(&exceptions, &count_id);
}

// ── S3 (guard): a SAFE count matching the ledger ────────────────────────────

#[tokio::test]
async fn safe_count_matching_the_ledger_writes_no_exception() {
    let db = Db::fresh().await;
    let branch = db.branch("safe_match").await;

    // The count finds the USD 1,000 the ledger holds.
    let count_id = count_the_safe(&db, &branch, CountKind::Safe, 10).await;

    let exceptions = db.cash_exceptions_for_count(&count_id).await;
    assert!(
        exceptions.is_empty(),
        "a SAFE count that matches the safe's ledger writes no cash exception: {exceptions:?}"
    );
}

/// Funds the branch's SAFE through the ledger (`BANK → SAFE`), as every LG-4 test funds a safe.
static FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

async fn fund_safe(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(BranchId::new(branch)),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "safe-count-variance-fund:{branch}:{currency}:{}",
                        FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund the safe through the ledger");
    tx.commit().await.expect("commit");
}
