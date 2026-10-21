//! Ticket #1487 (VT-1) — the rates §12.9 migration: `rate_policy_versions.value_tiers_on`,
//! `rate_policy_value_tiers`, `rate_policy_versions.sharp_move_alert_pct`, and
//! `org_starting_prices`.
//!
//! Written before the implementation by test-writer gemini/studio-10, ordered by the BE lane
//! controller claude/ad05bf8d. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Held to the seven rows of issue #1487 §9 (and `docs/design/blueprint/rates.md` §12.10 VT-1):
//! 1. Existing `rate_policy_versions` rows read `value_tiers_on = false` and `sharp_move_alert_pct IS NULL`.
//! 2. A threshold of 0 or less is refused.
//! 3. Four tiers with a rising upper are accepted.
//! 4. Tier 4 with an upper, or tier 1–3 without one, is refused by the CHECK.
//! 5. An upper of 0 or less is refused.
//! 6. `org_starting_prices` refuses `buy > sell`, an all-NULL row, and `buy ≤ 0` or `sell ≤ 0`.
//! 7. The duplicate `(organization_id, currency_code, version)` is refused.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Each test works in a PostgreSQL schema of its own.

use std::borrow::Cow;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "XC-B01";
/// Main's newest migration when VT-1 was written (`20261205121000_verification.sql`).
const BEFORE_VT1: i64 = 20261205121000;

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn new_schema() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("rt_mig");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let scoped = options.clone().options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(scoped)
            .await
            .expect("connect scoped to test schema");

        Self { pool, schema }
    }

    async fn migrate_through(&self, last: Option<i64>) {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"));
        let mut migrator = Migrator::new(dir)
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
        if let Some(last) = last {
            let kept: Vec<_> = migrator
                .migrations
                .iter()
                .filter(|m| m.version <= last)
                .cloned()
                .collect();
            migrator.migrations = Cow::Owned(kept);
        }
        migrator
            .run(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("apply api/migrations through {last:?}: {e}"));
    }

    async fn seed_branch(&self) {
        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ($1, 'Branch 01') ON CONFLICT DO NOTHING",
        )
        .bind(B01)
        .execute(&self.pool)
        .await
        .expect("seed branch B01");
    }

    /// Every migration applied.
    async fn start() -> Self {
        let db = Self::new_schema().await;
        db.migrate_through(None).await;
        db.seed_branch().await;
        db
    }

    /// Migrations applied only up to and including `last`.
    async fn start_before(last: i64) -> Self {
        let db = Self::new_schema().await;
        db.migrate_through(Some(last)).await;
        db.seed_branch().await;
        db
    }

    async fn insert_policy_version(&self, id: Uuid, version: i32) {
        sqlx::query(
            "INSERT INTO rate_policy_versions ( \
                id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                valid_from, approved_by, reason \
             ) VALUES ( \
                $1, $2, $3, 0.0, 0.0, '[]'::jsonb, \
                now(), 'admin', 'test policy' \
             )",
        )
        .bind(id)
        .bind(B01)
        .bind(version)
        .execute(&self.pool)
        .await
        .expect("insert rate_policy_versions");
    }

    async fn seed_reference_line(&self, batch_id: Uuid, line_id: Uuid, currency: &str) {
        let checksum = unique("chk");
        sqlx::query(
            "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES ($1, $2, CURRENT_DATE, now(), $3, 'COMPLETE')",
        )
        .bind(batch_id)
        .bind(format!("START:{ORG_1}"))
        .bind(checksum)
        .execute(&self.pool)
        .await
        .expect("insert rate_reference_batches");

        sqlx::query(
            "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
             VALUES ($1, $2, $3, 'START', 1.0, 35.0, 36.0, '{}'::jsonb)",
        )
        .bind(line_id)
        .bind(batch_id)
        .bind(currency)
        .execute(&self.pool)
        .await
        .expect("insert rate_reference_lines");
    }

    async fn finish(self) {
        let schema = self.schema;
        let pool = self.pool;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"
        )))
        .execute(&pool)
        .await
        .expect("drop test schema");
        pool.close().await;
    }
}

fn assert_check_violation(err: &sqlx::Error) {
    match err {
        sqlx::Error::Database(db_err) => {
            assert_eq!(
                db_err.code().as_deref(),
                Some("23514"),
                "expected check_violation (23514), got: {db_err}"
            );
        }
        other => panic!("expected database check_violation error, got: {other:?}"),
    }
}

fn assert_unique_violation(err: &sqlx::Error) {
    match err {
        sqlx::Error::Database(db_err) => {
            assert_eq!(
                db_err.code().as_deref(),
                Some("23505"),
                "expected unique_violation (23505), got: {db_err}"
            );
        }
        other => panic!("expected database unique_violation error, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. Existing rate_policy_versions rows read value_tiers_on = false and sharp_move_alert_pct IS NULL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_existing_rate_policy_versions_read_default_false_and_null_threshold() {
    // 1. Run migrations only up to main's newest migration before VT-1
    let db = TestDb::start_before(BEFORE_VT1).await;
    let existing_policy_id = Uuid::new_v4();
    db.insert_policy_version(existing_policy_id, 1).await;

    // 2. Now run the remaining migrations (including VT-1 when present)
    db.migrate_through(None).await;

    // 3. Query the row that EXISTED before VT-1 ran
    let (value_tiers_on, sharp_move_alert_pct): (bool, Option<Decimal>) = sqlx::query_as(
        "SELECT value_tiers_on, sharp_move_alert_pct FROM rate_policy_versions WHERE id = $1",
    )
    .bind(existing_policy_id)
    .fetch_one(&db.pool)
    .await
    .expect("query existing rate_policy_versions for value_tiers_on and sharp_move_alert_pct");

    assert!(
        !value_tiers_on,
        "existing rate_policy_versions row must read value_tiers_on = false"
    );
    assert!(
        sharp_move_alert_pct.is_none(),
        "existing rate_policy_versions row must read sharp_move_alert_pct IS NULL"
    );

    // 4. Also verify that a new row inserted after the migration defaults to false and NULL
    let new_policy_id = Uuid::new_v4();
    db.insert_policy_version(new_policy_id, 2).await;

    let (new_value_tiers_on, new_sharp_move_alert_pct): (bool, Option<Decimal>) = sqlx::query_as(
        "SELECT value_tiers_on, sharp_move_alert_pct FROM rate_policy_versions WHERE id = $1",
    )
    .bind(new_policy_id)
    .fetch_one(&db.pool)
    .await
    .expect("query new rate_policy_versions for default value_tiers_on and sharp_move_alert_pct");

    assert!(
        !new_value_tiers_on,
        "new rate_policy_versions row must default value_tiers_on to false"
    );
    assert!(
        new_sharp_move_alert_pct.is_none(),
        "new rate_policy_versions row must default sharp_move_alert_pct to NULL"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// 2. A threshold of 0 or less is refused
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sharp_move_alert_pct_zero_or_less_is_refused() {
    let db = TestDb::start().await;
    let policy_id = Uuid::new_v4();
    db.insert_policy_version(policy_id, 1).await;

    // A positive threshold must be accepted
    sqlx::query("UPDATE rate_policy_versions SET sharp_move_alert_pct = 2.5000 WHERE id = $1")
        .bind(policy_id)
        .execute(&db.pool)
        .await
        .expect("positive sharp_move_alert_pct must be accepted");

    // Threshold of 0 must be refused by CHECK (sharp_move_alert_pct IS NULL OR sharp_move_alert_pct > 0)
    let err_zero =
        sqlx::query("UPDATE rate_policy_versions SET sharp_move_alert_pct = 0.0000 WHERE id = $1")
            .bind(policy_id)
            .execute(&db.pool)
            .await
            .expect_err("sharp_move_alert_pct = 0 must be refused by CHECK");
    assert_check_violation(&err_zero);

    // Negative threshold must be refused by CHECK
    let err_neg =
        sqlx::query("UPDATE rate_policy_versions SET sharp_move_alert_pct = -1.0000 WHERE id = $1")
            .bind(policy_id)
            .execute(&db.pool)
            .await
            .expect_err("sharp_move_alert_pct < 0 must be refused by CHECK");
    assert_check_violation(&err_neg);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// 3. Four tiers with a rising upper are accepted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_four_tiers_with_rising_upper_are_accepted() {
    let db = TestDb::start().await;
    let policy_id = Uuid::new_v4();
    db.insert_policy_version(policy_id, 1).await;

    // Insert 4 tiers with rising upper bounds:
    // Tier 1: upper 50,000.00 THB
    // Tier 2: upper 100,000.00 THB
    // Tier 3: upper 200,000.00 THB
    // Tier 4: upper NULL (open above)
    let tiers_to_insert = [
        (
            1i16,
            Some(Decimal::new(5000000, 2)),
            Decimal::new(500, 4),
            Decimal::new(500, 4),
        ),
        (
            2i16,
            Some(Decimal::new(10000000, 2)),
            Decimal::new(400, 4),
            Decimal::new(400, 4),
        ),
        (
            3i16,
            Some(Decimal::new(20000000, 2)),
            Decimal::new(300, 4),
            Decimal::new(300, 4),
        ),
        (4i16, None, Decimal::new(200, 4), Decimal::new(200, 4)),
    ];

    for (tier_no, upper, buy_margin, sell_margin) in tiers_to_insert {
        sqlx::query(
            "INSERT INTO rate_policy_value_tiers ( \
                policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
             ) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(policy_id)
        .bind(tier_no)
        .bind(upper)
        .bind(buy_margin)
        .bind(sell_margin)
        .execute(&db.pool)
        .await
        .expect("inserting valid tier must succeed");
    }

    let rows: Vec<(i16, Option<Decimal>, Decimal, Decimal)> = sqlx::query_as(
        "SELECT tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         FROM rate_policy_value_tiers \
         WHERE policy_version_id = $1 \
         ORDER BY tier_no",
    )
    .bind(policy_id)
    .fetch_all(&db.pool)
    .await
    .expect("query rate_policy_value_tiers");

    assert_eq!(rows.len(), 4, "must read back exactly four tiers");
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, Some(Decimal::new(5000000, 2)));
    assert_eq!(rows[1].0, 2);
    assert_eq!(rows[1].1, Some(Decimal::new(10000000, 2)));
    assert_eq!(rows[2].0, 3);
    assert_eq!(rows[2].1, Some(Decimal::new(20000000, 2)));
    assert_eq!(rows[3].0, 4);
    assert_eq!(rows[3].1, None, "tier 4 upper must be NULL");

    // Duplicate (policy_version_id, tier_no) is refused by PRIMARY KEY
    let err_dup = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 1, 60000.00, 0.0500, 0.0500)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("duplicate (policy_version_id, tier_no) must be refused by PRIMARY KEY");
    assert_unique_violation(&err_dup);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// 4. Tier 4 with an upper, or tier 1–3 without one, is refused by the CHECK
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tier_4_with_upper_or_tier_1_to_3_without_upper_is_refused_by_check() {
    let db = TestDb::start().await;
    let policy_id = Uuid::new_v4();
    db.insert_policy_version(policy_id, 1).await;

    // CHECK ((tier_no = 4) = (upper_inclusive_thb IS NULL))

    // Tier 4 with an upper is refused:
    let err_t4 = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 4, 500000.00, 0.0200, 0.0200)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier 4 with upper must be refused by CHECK");
    assert_check_violation(&err_t4);

    // Tier 1 without an upper (NULL) is refused:
    let err_t1 = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 1, NULL, 0.0500, 0.0500)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier 1 without upper must be refused by CHECK");
    assert_check_violation(&err_t1);

    // Tier 2 without an upper (NULL) is refused:
    let err_t2 = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 2, NULL, 0.0400, 0.0400)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier 2 without upper must be refused by CHECK");
    assert_check_violation(&err_t2);

    // Tier 3 without an upper (NULL) is refused:
    let err_t3 = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 3, NULL, 0.0300, 0.0300)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier 3 without upper must be refused by CHECK");
    assert_check_violation(&err_t3);

    // Also tier_no outside 1..4 (e.g. tier 0 or tier 5) is refused by CHECK (tier_no BETWEEN 1 AND 4):
    let err_t0 = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 0, 10000.00, 0.0500, 0.0500)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier 0 must be refused by CHECK");
    assert_check_violation(&err_t0);

    let err_t5 = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 5, NULL, 0.0500, 0.0500)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier 5 must be refused by CHECK");
    assert_check_violation(&err_t5);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// 5. An upper of 0 or less is refused
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rate_policy_value_tier_upper_zero_or_less_is_refused() {
    let db = TestDb::start().await;
    let policy_id = Uuid::new_v4();
    db.insert_policy_version(policy_id, 1).await;

    // CHECK (upper_inclusive_thb IS NULL OR upper_inclusive_thb > 0)

    // upper = 0 is refused:
    let err_zero = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 1, 0.00, 0.0500, 0.0500)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier upper = 0 must be refused by CHECK");
    assert_check_violation(&err_zero);

    // upper < 0 is refused:
    let err_neg = sqlx::query(
        "INSERT INTO rate_policy_value_tiers ( \
            policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
         ) VALUES ($1, 1, -500.00, 0.0500, 0.0500)",
    )
    .bind(policy_id)
    .execute(&db.pool)
    .await
    .expect_err("tier upper < 0 must be refused by CHECK");
    assert_check_violation(&err_neg);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// 6. org_starting_prices refuses buy > sell, an all-NULL row, and buy ≤ 0 or sell ≤ 0
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_org_starting_prices_refuses_buy_gt_sell_all_null_and_non_positive_prices() {
    let db = TestDb::start().await;
    let org_id = Uuid::parse_str(ORG_1).unwrap();
    let batch_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    db.seed_reference_line(batch_id, line_id, "USD").await;

    // Valid row with both buy and sell (buy <= sell, both > 0): accepted
    let valid_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 1, 1.0, 35.0, 36.0, 'owner', 'seed valid', $3)",
    )
    .bind(valid_id)
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect("valid buy <= sell must be accepted");

    // Valid row with buy only (sell IS NULL): accepted
    let buy_only_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 2, 1.0, 35.0, NULL, 'owner', 'seed buy only', $3)",
    )
    .bind(buy_only_id)
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect("valid buy-only row must be accepted");

    // Valid row with sell only (buy IS NULL): accepted
    let sell_only_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 3, 1.0, NULL, 36.0, 'owner', 'seed sell only', $3)",
    )
    .bind(sell_only_id)
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect("valid sell-only row must be accepted");

    // CHECK (buy IS NULL OR sell IS NULL OR buy <= sell)
    // buy > sell: refused
    let err_buy_gt_sell = sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 4, 1.0, 37.0, 36.0, 'owner', 'buy > sell', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect_err("buy > sell must be refused by CHECK");
    assert_check_violation(&err_buy_gt_sell);

    // CHECK (buy IS NOT NULL OR sell IS NOT NULL)
    // all-NULL row (buy IS NULL AND sell IS NULL): refused
    let err_all_null = sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 5, 1.0, NULL, NULL, 'owner', 'all null', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect_err("all-NULL row must be refused by CHECK");
    assert_check_violation(&err_all_null);

    // CHECK (buy IS NULL OR buy > 0)
    // buy = 0: refused (with sell = NULL so only buy > 0 or buy <= sell could apply, but sell is NULL so buy <= sell is trivially true)
    let err_buy_zero = sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 6, 1.0, 0.0, 36.0, 'owner', 'buy = 0', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect_err("buy = 0 must be refused by CHECK");
    assert_check_violation(&err_buy_zero);

    // buy < 0: refused
    let err_buy_neg = sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 7, 1.0, -10.0, 36.0, 'owner', 'buy < 0', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect_err("buy < 0 must be refused by CHECK");
    assert_check_violation(&err_buy_neg);

    // CHECK (sell IS NULL OR sell > 0)
    // Must send buy as NULL so buy <= sell is trivially satisfied, isolating sell > 0 as the sole refusal reason
    // sell = 0: refused
    let err_sell_zero = sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 8, 1.0, NULL, 0.0, 'owner', 'sell = 0', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect_err("sell = 0 must be refused by CHECK");
    assert_check_violation(&err_sell_zero);

    // sell < 0: refused
    let err_sell_neg = sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 9, 1.0, NULL, -10.0, 'owner', 'sell < 0', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect_err("sell < 0 must be refused by CHECK");
    assert_check_violation(&err_sell_neg);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// 7. The duplicate (organization_id, currency_code, version) is refused
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_org_starting_prices_refuses_duplicate_org_currency_version() {
    let db = TestDb::start().await;
    let org_id = Uuid::parse_str(ORG_1).unwrap();
    let batch_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    db.seed_reference_line(batch_id, line_id, "USD").await;

    // First row with (org_id, 'USD', 1): accepted
    let first_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 1, 1.0, 35.0, 36.0, 'owner', 'version 1', $3)",
    )
    .bind(first_id)
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect("first (org_id, 'USD', 1) row must be accepted");

    // Second row with same (org_id, 'USD', 1) but different id: refused by UNIQUE (organization_id, currency_code, version)
    let second_id = Uuid::new_v4();
    let err_dup = sqlx::query(
        "INSERT INTO org_starting_prices ( \
            id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, reason, reference_line_id \
         ) VALUES ($1, $2, 'USD', 1, 1.0, 35.5, 36.5, 'owner', 'version 1 duplicate', $3)",
    )
    .bind(second_id)
    .bind(org_id)
    .bind(line_id)
    .execute(&db.pool)
    .await
    .expect_err("duplicate (org_id, 'USD', 1) must be refused by UNIQUE constraint");
    assert_unique_violation(&err_dup);

    db.finish().await;
}
