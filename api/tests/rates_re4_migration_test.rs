//! Ticket #601 (RE-4) — migration 5 (`20261010125000_published_rate_line_ids.sql`): the line-id
//! columns, the conversion of stored JPY/KRW rates to per-`unit_size`, its before-deploy count,
//! and the design's deploy gate.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §8 migration 5:
//! - adds `published_rate_line_id uuid NULL REFERENCES published_rate_lines(id)` to
//!   `transactions`, `kiosk_requests` and `bookings`; existing rows get `NULL` (no backfill);
//! - converts the stored rates in the same migration:
//!   `UPDATE <table> t SET rate = t.rate * c.unit_size, unit_size = c.unit_size FROM currencies c
//!   WHERE c.code = t.<currency column> AND c.unit_size <> 1 AND t.unit_size = 1`, with
//!   `transactions.reference_rate` multiplied the same way; the currency column is `code` on
//!   `transactions` and `bookings`, `currency` on `kiosk_requests`;
//! - stored principals do not change, because `amount × (rate × u) ÷ u` is the same THB;
//! - "Count before deploy" (the SQL below, verbatim) counts what is still per 1;
//! - the deploy gate (the SQL below, verbatim) counts the (branch, code) pairs whose newest
//!   direct-save row, saved after the branch's first publication, differs from (or has no) line
//!   in the current publication; the worked case of PR 594's review.
//! - §3.2 (`ExchangeArithmetic::thb`); the ticket's edge case "a JPY/KRW row stored per 1 before
//!   migration 5 reading as the same THB after it"; §9 RE-4.
//!
//! The conversion test applies every migration older than migration 5, writes rows the way the
//! pre-RE-4 binary writes them (per 1, `unit_size` 1, `numeric(20,8)` rates since RU-1), then
//! applies the rest. The gate test runs on a fully migrated schema: the gate reads only `rates`,
//! `currencies`, `branches`, `rate_publications` and `published_rate_lines`, which migration 5
//! does not change. Publications are made through the rate engine (`rates_re4_common`).
//!
//! `down` is not run here: like migrations 1 and 4, migration 5's `down` is a comment in the file
//! (sqlx applies no down section from a plain `.sql` file), so no test can apply it.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Each test works in a schema of its own.
//!
//! Signatures this test assumes:
//! - [required] the file `api/migrations/20261010125000_published_rate_line_ids.sql`
//!   (version `20261010125000`);
//! - [required] the columns `transactions.published_rate_line_id`,
//!   `kiosk_requests.published_rate_line_id`, `bookings.published_rate_line_id`, `uuid`, nullable,
//!   each with a foreign key to `published_rate_lines(id)`;
//! - `xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Rate, UnitSize}` (RU-1, merged).

mod rates_re4_common;

use std::borrow::Cow;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use rates_re4_common::{d, publish};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Rate, UnitSize};

const MIGRATION_5: i64 = 20261010125000;
const MUST_FINISH: Duration = Duration::from_secs(30);

/// §8 migration 5, "Count before deploy", verbatim.
const COUNT_BEFORE_DEPLOY: &str = "\
SELECT 'transactions', count(*) FROM transactions WHERE code IN ('JPY','KRW') AND unit_size = 1 UNION ALL
SELECT 'kiosk_requests', count(*) FROM kiosk_requests WHERE currency IN ('JPY','KRW') AND unit_size = 1 UNION ALL
SELECT 'bookings', count(*) FROM bookings WHERE code IN ('JPY','KRW') AND unit_size = 1;";

/// §8 migration 5, "Deploy gate", verbatim.
const DEPLOY_GATE: &str = "\
-- per (branch, code): the newest direct-save row saved after the branch's first publication,
-- against that code's line in the branch's current publication
WITH latest AS (
  SELECT DISTINCT ON (r.branch_id, r.code) r.branch_id, r.code, r.buy, r.sell, r.updated_at
    FROM rates r ORDER BY r.branch_id, r.code, r.id DESC
), first_pub AS (
  SELECT branch_id, min(published_at) AS at FROM rate_publications GROUP BY branch_id
)
SELECT count(*)
  FROM latest l
  JOIN currencies c ON c.code = l.code
  JOIN branches b ON b.id = l.branch_id
  LEFT JOIN first_pub f ON f.branch_id = l.branch_id
  LEFT JOIN published_rate_lines pl
    ON pl.publication_id = b.current_publication_id
   AND pl.currency_code = l.code AND pl.denomination_group = 'ALL'
 WHERE (f.at IS NULL OR l.updated_at > f.at)                    -- saved inside the window
   AND (pl.id IS NULL                                           -- no line: the currency would vanish
        OR pl.buy_rate  IS DISTINCT FROM l.buy  * c.unit_size   -- the line says something else
        OR pl.sell_rate IS DISTINCT FROM l.sell * c.unit_size);";

// ------------------------------------------------------------ database

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn test_database() -> PgConnectOptions {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             The migration 5 tests need a real PostgreSQL and do not skip. Point it at an \
             existing database whose name ends in `_test`, e.g.\n\
             TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
        )
    });
    let options = PgConnectOptions::from_str(&url)
        .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
    let database = options.get_database().unwrap_or("").to_owned();
    assert!(
        database.ends_with("_test"),
        "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
    );
    options
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    /// A pool confined to a fresh, empty schema.
    async fn empty() -> Self {
        let options = test_database();
        let schema = unique("re4mig");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
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
        Self { pool, schema }
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

async fn migrator() -> Migrator {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    Migrator::new(dir.as_path())
        .await
        .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
}

/// The migrations older than `version`.
async fn before(version: i64) -> Migrator {
    let all = migrator().await;
    let mut older = migrator().await;
    older.migrations = Cow::Owned(
        all.migrations
            .iter()
            .filter(|m| m.version < version)
            .cloned()
            .collect(),
    );
    older
}

async fn count_before_deploy(pool: &PgPool) -> Vec<(String, i64)> {
    sqlx::query_as(COUNT_BEFORE_DEPLOY)
        .fetch_all(pool)
        .await
        .expect("the count before deploy runs")
}

async fn gate(pool: &PgPool) -> i64 {
    sqlx::query_scalar(DEPLOY_GATE)
        .fetch_one(pool)
        .await
        .expect("the deploy gate runs")
}

// ------------------------------------------------------------ rows as the pre-RE-4 binary writes them

async fn customer(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO customers (id, branch_id, name, passport_no, nationality, mode, created_at) \
         VALUES ('CU-1', 'MAIN', 'Somchai', 'P-1', 'THA', 'DEMO', now())",
    )
    .execute(pool)
    .await
    .expect("insert a customer");
}

/// A DEMO exchange (no shift), as the pre-RE-4 save writes it: `unit_size` left at its default.
#[allow(clippy::too_many_arguments)]
async fn transaction(
    pool: &PgPool,
    document_ref: &str,
    side: &str,
    code: &str,
    amount: &str,
    rate: &str,
    reference_rate: Option<&str>,
    principal: &str,
    unit_size: Option<&str>,
) {
    let (received, paid) = if side == "SELL" {
        (principal, "0")
    } else {
        ("0", principal)
    };
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, \
            payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, \
            reference_rate, principal, fee, bank_fee, received, paid, method, payment_ref, account, \
            staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by) \
         VALUES ($1, 'MAIN', 'DEMO', NULL, $1, 'hash', 'CU-1', 'Somchai', 'THA', $2, $3, \
            $4::numeric, $5::numeric, $6::numeric, $7::numeric, 0, 0, $8::numeric, $9::numeric, \
            'Cash', '', 'Cash drawer 1', 'owner', 'WALK-IN', '', '2026-10-09', now(), now(), 'owner')",
    )
    .bind(document_ref)
    .bind(side)
    .bind(code)
    .bind(amount)
    .bind(rate)
    .bind(reference_rate)
    .bind(principal)
    .bind(received)
    .bind(paid)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("insert transaction {document_ref}: {e}"));
    if let Some(unit) = unit_size {
        sqlx::query("UPDATE transactions SET unit_size = $2::numeric WHERE document_ref = $1")
            .bind(document_ref)
            .bind(unit)
            .execute(pool)
            .await
            .expect("set the unit size");
    }
}

async fn kiosk_request(
    pool: &PgPool,
    id: &str,
    currency: &str,
    rate: &str,
    foreign: &str,
    thb: &str,
) {
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
            rate, foreign_amount, thb, input_amount, payment_method) \
         VALUES ('MAIN', $1, 'QUOTED', now(), NULL, 'BUY', $2, $3::numeric, $4::numeric, \
            $5::numeric, $4::numeric, 'Cash')",
    )
    .bind(id)
    .bind(currency)
    .bind(rate)
    .bind(foreign)
    .bind(thb)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("insert kiosk request {id}: {e}"));
}

async fn booking(pool: &PgPool, id: &str, code: &str, amount: &str, rate: &str) {
    sqlx::query(
        "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
            type, code, amount, rate, status, created_at, created_by, expires_at) \
         VALUES ($1, 'MAIN', $1, 'hash', 'Ann', '0812345678', NULL, 'SELL', $2, $3::numeric, \
            $4::numeric, 'CONFIRMED', now(), 'owner', now() + interval '30 minutes')",
    )
    .bind(id)
    .bind(code)
    .bind(amount)
    .bind(rate)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("insert booking {id}: {e}"));
}

/// `(rate, reference_rate, unit_size, principal, line id)` of one exchange.
async fn transaction_after(
    pool: &PgPool,
    document_ref: &str,
) -> (Decimal, Option<Decimal>, Decimal, Decimal, Option<String>) {
    sqlx::query_as(
        "SELECT rate, reference_rate, unit_size, principal, published_rate_line_id::text \
         FROM transactions WHERE document_ref = $1",
    )
    .bind(document_ref)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("transaction {document_ref}: {e}"))
}

/// `(rate, unit_size, thb, line id)` of one kiosk request.
async fn kiosk_after(pool: &PgPool, id: &str) -> (Decimal, Decimal, Decimal, Option<String>) {
    sqlx::query_as(
        "SELECT rate, unit_size, thb, published_rate_line_id::text FROM kiosk_requests \
         WHERE branch_id = 'MAIN' AND id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("kiosk request {id}: {e}"))
}

/// `(rate, unit_size, amount, line id)` of one booking.
async fn booking_after(pool: &PgPool, id: &str) -> (Decimal, Decimal, Decimal, Option<String>) {
    sqlx::query_as(
        "SELECT rate, unit_size, amount, published_rate_line_id::text FROM bookings WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("booking {id}: {e}"))
}

/// THB of `amount` at `rate` per `unit` through the one arithmetic (§3.2).
fn thb(amount: Decimal, rate: Decimal, unit: Decimal) -> Decimal {
    let amount = ForeignAmount::from_decimal(amount);
    let rate = Rate::parse(&rate.normalize().to_string()).expect("a stored rate parses");
    let unit = UnitSize::parse(&unit.normalize().to_string()).expect("a stored unit parses");
    d(&ExchangeArithmetic::thb(amount, rate, unit).to_string())
}

// ------------------------------------------------------------ tests

/// §8 migration 5: the per-1 JPY and KRW rates of the three tables become per `unit_size`
/// (`transactions.reference_rate` too), the count before deploy goes from 4 rows to 0, USD and a
/// row already per 100 are untouched, every existing row gets a NULL line id, and each converted
/// row reads as the same THB through `ExchangeArithmetic::thb` as it did per 1.
#[tokio::test]
async fn migration_5_converts_stored_jpy_and_krw_rates_to_their_unit_size() {
    let db = TestDb::empty().await;
    let all = migrator().await;
    assert!(
        all.migrations.iter().any(|m| m.version == MIGRATION_5),
        "api/migrations has migration 5, 20261010125000_published_rate_line_ids.sql"
    );
    before(MIGRATION_5)
        .await
        .run(&db.pool)
        .await
        .expect("apply the migrations before migration 5");

    customer(&db.pool).await;
    // per 1, as the pre-RE-4 binary stores them
    transaction(
        &db.pool,
        "T-JPY",
        "SELL",
        "JPY",
        "1000",
        "0.235",
        Some("0.2301"),
        "235.00",
        None,
    )
    .await;
    transaction(
        &db.pool,
        "T-KRW",
        "BUY",
        "KRW",
        "100000",
        "0.02512345",
        None,
        "2512.35",
        None,
    )
    .await;
    transaction(
        &db.pool,
        "T-USD",
        "BUY",
        "USD",
        "100",
        "32.1",
        Some("32"),
        "3210.00",
        None,
    )
    .await;
    // already per 100 (the guard `t.unit_size = 1` keeps it from being multiplied twice)
    transaction(
        &db.pool,
        "T-JPY-100",
        "SELL",
        "JPY",
        "1000",
        "23.5",
        None,
        "235.00",
        Some("100"),
    )
    .await;
    kiosk_request(&db.pool, "K-JPY", "JPY", "0.231", "1000", "231.00").await;
    kiosk_request(&db.pool, "K-USD", "USD", "33", "100", "3300.00").await;
    booking(&db.pool, "BK-KRW", "KRW", "50000", "0.026").await;
    booking(&db.pool, "BK-USD", "USD", "1000", "36.25").await;

    assert_eq!(
        count_before_deploy(&db.pool).await,
        vec![
            ("transactions".to_owned(), 2),
            ("kiosk_requests".to_owned(), 1),
            ("bookings".to_owned(), 1),
        ],
        "the count before deploy sees the per-1 JPY/KRW rows"
    );

    all.run(&db.pool)
        .await
        .expect("apply migration 5 and the rest");

    assert_eq!(
        count_before_deploy(&db.pool).await,
        vec![
            ("transactions".to_owned(), 0),
            ("kiosk_requests".to_owned(), 0),
            ("bookings".to_owned(), 0),
        ],
        "nothing is per 1 any more"
    );

    let jpy = transaction_after(&db.pool, "T-JPY").await;
    assert_eq!(
        jpy,
        (d("23.5"), Some(d("23.01")), d("100"), d("235.00"), None),
        "JPY ×100, reference too; principal unchanged; no line"
    );
    let krw = transaction_after(&db.pool, "T-KRW").await;
    assert_eq!(
        krw,
        (d("25.12345"), None, d("1000"), d("2512.35"), None),
        "KRW ×1000; a NULL reference stays NULL"
    );
    let usd = transaction_after(&db.pool, "T-USD").await;
    assert_eq!(usd, (d("32.1"), Some(d("32")), d("1"), d("3210.00"), None));
    let already = transaction_after(&db.pool, "T-JPY-100").await;
    assert_eq!(
        already,
        (d("23.5"), None, d("100"), d("235.00"), None),
        "a row already per 100 is not multiplied again"
    );

    assert_eq!(
        kiosk_after(&db.pool, "K-JPY").await,
        (d("23.1"), d("100"), d("231.00"), None)
    );
    assert_eq!(
        kiosk_after(&db.pool, "K-USD").await,
        (d("33"), d("1"), d("3300.00"), None)
    );
    assert_eq!(
        booking_after(&db.pool, "BK-KRW").await,
        (d("26"), d("1000"), d("50000"), None)
    );
    assert_eq!(
        booking_after(&db.pool, "BK-USD").await,
        (d("36.25"), d("1"), d("1000"), None)
    );

    // the same THB before and after: amount × (rate × u) ÷ u
    assert_eq!(
        thb(d("1000"), jpy.0, jpy.2),
        d("235.00"),
        "T-JPY reads 235.00"
    );
    assert_eq!(
        thb(d("100000"), krw.0, krw.2),
        d("2512.35"),
        "T-KRW reads 2512.35"
    );
    let (k_rate, k_unit, k_thb, _) = kiosk_after(&db.pool, "K-JPY").await;
    assert_eq!(thb(d("1000"), k_rate, k_unit), k_thb, "K-JPY reads 231.00");
    let (b_rate, b_unit, b_amount, _) = booking_after(&db.pool, "BK-KRW").await;
    assert_eq!(
        thb(b_amount, b_rate, b_unit),
        d("1300.00"),
        "BK-KRW: 50000 × 0.026 = 1300.00 per 1, the same per 1000"
    );
    db.finish().await;
}

/// §8 migration 5: the three columns are nullable uuids with a foreign key to
/// `published_rate_lines(id)`; a line id that names no line is refused.
#[tokio::test]
async fn migration_5_adds_a_nullable_line_id_referencing_published_rate_lines() {
    let db = TestDb::empty().await;
    migrator()
        .await
        .run(&db.pool)
        .await
        .expect("apply api/migrations");

    for table in ["transactions", "kiosk_requests", "bookings"] {
        let column: Option<(String, String)> = sqlx::query_as(
            "SELECT data_type, is_nullable FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 \
               AND column_name = 'published_rate_line_id'",
        )
        .bind(table)
        .fetch_optional(&db.pool)
        .await
        .expect("read information_schema");
        assert_eq!(
            column,
            Some(("uuid".to_owned(), "YES".to_owned())),
            "{table}.published_rate_line_id is a nullable uuid"
        );
        let references: Vec<String> = sqlx::query_scalar(
            "SELECT confrel.relname::text FROM pg_constraint con \
               JOIN pg_class rel ON rel.oid = con.conrelid \
               JOIN pg_class confrel ON confrel.oid = con.confrelid \
               JOIN pg_namespace ns ON ns.oid = rel.relnamespace \
               JOIN pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = ANY (con.conkey) \
             WHERE con.contype = 'f' AND ns.nspname = current_schema() AND rel.relname = $1 \
               AND att.attname = 'published_rate_line_id'",
        )
        .bind(table)
        .fetch_all(&db.pool)
        .await
        .expect("read pg_constraint");
        assert_eq!(
            references,
            vec!["published_rate_lines".to_owned()],
            "{table}.published_rate_line_id references published_rate_lines"
        );
    }

    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, rate, \
            foreign_amount, thb, input_amount, payment_method) \
         VALUES ('MAIN', 'K-1', 'QUOTED', now(), 'BUY', 'USD', 33, 100, 3300, 100, 'Cash')",
    )
    .execute(&db.pool)
    .await
    .expect("insert a request");
    let dangling = sqlx::query(
        "UPDATE kiosk_requests SET published_rate_line_id = gen_random_uuid() WHERE id = 'K-1'",
    )
    .execute(&db.pool)
    .await;
    assert!(
        dangling.is_err(),
        "a line id that names no published line is refused"
    );
    db.finish().await;
}

/// The worked case of §8's deploy gate (PR 594 review): publication 1 holds JPY = X; staff then
/// save JPY = Y directly; a publication that changes only USD keeps X, so the gate still counts
/// JPY; publishing Y clears it. A direct-save row older than the first publication is never
/// counted. The gate's SQL is the design's, verbatim: this test is what lets the PR body carry
/// it as measured against the schema.
#[tokio::test]
async fn the_deploy_gate_counts_a_direct_save_the_current_line_does_not_say() {
    let db = TestDb::empty().await;
    migrator()
        .await
        .run(&db.pool)
        .await
        .expect("apply api/migrations");
    let save = |code: &'static str, buy: &'static str, sell: &'static str, at: DateTime<Utc>| {
        let pool = db.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
                 VALUES ('MAIN', $1, $2::numeric, $3::numeric, $4, 'owner', $5)",
            )
            .bind(code)
            .bind(buy)
            .bind(sell)
            .bind(at)
            .bind(unique("req"))
            .execute(&pool)
            .await
            .expect("a direct save");
        }
    };

    // before any publication: a direct save, outside any window that has begun
    save(
        "JPY",
        "0.22",
        "0.225",
        Utc::now() - chrono::Duration::days(1),
    )
    .await;
    assert_eq!(
        gate(&db.pool).await,
        1,
        "no publication: the currency would vanish"
    );

    publish(
        &db.pool,
        "MAIN",
        &[("JPY", "23.1", "23.5"), ("USD", "32.1", "33")],
    )
    .await;
    assert_eq!(
        gate(&db.pool).await,
        0,
        "rows older than the first publication are its source, not counted"
    );

    // staff save JPY = Y directly after publication 1
    save(
        "JPY",
        "0.24",
        "0.245",
        Utc::now() + chrono::Duration::minutes(1),
    )
    .await;
    assert_eq!(
        gate(&db.pool).await,
        1,
        "the line says X, the readers quote Y"
    );

    publish(
        &db.pool,
        "MAIN",
        &[("JPY", "23.1", "23.5"), ("USD", "32.2", "33")],
    )
    .await;
    assert_eq!(
        gate(&db.pool).await,
        1,
        "a publication that changes only USD clears nothing"
    );

    publish(
        &db.pool,
        "MAIN",
        &[("JPY", "24", "24.5"), ("USD", "32.2", "33")],
    )
    .await;
    assert_eq!(gate(&db.pool).await, 0, "the line now says Y × 100");
    db.finish().await;
}
