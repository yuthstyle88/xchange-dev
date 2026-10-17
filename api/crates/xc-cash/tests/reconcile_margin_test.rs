//! Up-front tests for LG-7.4 (#1058; design `docs/design/blueprint/ledger-inventory.md` §6 check 4,
//! §6.1): margin equals sales minus cost. Written by the ticket author before the implementation.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it
//! does not skip.

use xc_cash::application::reconcile::margin::{MarginCheck, QUERY};

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_cash::application::reconcile::{Offender, Reconcile, ReconcileCheck, ReconcileScope};
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_ORG: &str = "00000000-0000-0000-0000-0000000000f2";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// A fresh schema with every migration applied; branches `B1` and `B2` of the organisation, and
/// `X1` of another organisation.
async fn db() -> PgPool {
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
    let schema = unique("lg7");
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
        .max_connections(4)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    Migrator::new(migrations_dir().as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1::uuid, 'OTHER', 'Asia/Bangkok', now())",
    )
    .bind(OTHER_ORG)
    .execute(&pool)
    .await
    .expect("another organisation");
    for (id, org) in [("B1", ORG), ("B2", ORG), ("X1", OTHER_ORG)] {
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(id)
            .bind(org)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("branch {id}: {e}"));

        let customer_id = format!("cust-{id}");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $3, 'Customer', 'LIVE', now())",
        )
        .bind(&customer_id)
        .bind(org)
        .bind(format!("no-{id}"))
        .execute(&pool)
        .await
        .unwrap_or_else(|e| panic!("customer {id}: {e}"));

        let teller_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, $3, 'ACTIVE', 'test')",
        )
        .bind(teller_id)
        .bind(org)
        .bind(format!("teller-{id}"))
        .execute(&pool)
        .await
        .unwrap_or_else(|e| panic!("user {id}: {e}"));

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id, organization_id) \
             VALUES ($1, $2, 'OPEN', now(), 'owner', $3, $4, $5::uuid)",
        )
        .bind(format!("shift-{id}"))
        .bind(id)
        .bind(format!("req-{id}"))
        .bind(teller_id)
        .bind(org)
        .execute(&pool)
        .await
        .unwrap_or_else(|e| panic!("shift {id}: {e}"));
    }
    pool
}

fn scope(branch: Option<&str>, period: Option<(NaiveDate, NaiveDate)>) -> ReconcileScope {
    ReconcileScope {
        organization_id: ORG.to_string(),
        branch: branch.map(BranchId::new),
        period,
    }
}

/// Runs `check` alone in a read-only transaction.
async fn offenders(
    pool: &PgPool,
    check: &dyn ReconcileCheck,
    scope: &ReconcileScope,
) -> Vec<Offender> {
    let mut tx = pool.begin().await.expect("begin");
    let out = check.run_in(&mut tx, scope).await.expect("the check runs");
    drop(tx);
    out
}

fn keys(offenders: &[Offender]) -> Vec<String> {
    offenders.iter().map(|o| o.key.clone()).collect()
}

/// `standard()` registers check `id`.
async fn assert_registered(pool: &PgPool, id: u8) {
    let report = Reconcile::standard()
        .run(pool, &scope(None, None))
        .await
        .expect("the standard run");
    assert!(
        report.checks.iter().any(|c| c.id == id),
        "check {id} is registered in Reconcile::standard()"
    );
}

/// 101 offenders give 100 plus the marker.
fn assert_capped(found: &[Offender]) {
    assert_eq!(found.len(), 101, "100 offenders plus the marker");
    assert_eq!(
        found[100].key, "truncated",
        "the marker is last: {:?}",
        found[100]
    );
    assert!(found[..100].iter().all(|o| o.key != "truncated"));
}

#[allow(clippy::too_many_arguments)]
async fn sell_line(
    pool: &PgPool,
    org: &str,
    branch: &str,
    date: NaiveDate,
    currency: &str,
    thb_amount: &str,
    cost_basis_thb: Option<&str>,
    realized_margin_thb: Option<&str>,
) -> String {
    let tx_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    let sub_id = unique("sub");
    let doc_ref = unique("TX");
    let recorded_at = date.and_hms_opt(10, 0, 0).unwrap().and_utc();

    let shift = format!("shift-{branch}");
    let customer_id = format!("cust-{branch}");

    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
             customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
             recorded_by, completed_at, status, organization_id) VALUES ($1, $2, $3, 'LIVE', $4, $5, 'hash', $6, 'Customer', 'THA', 'staff', '', '', $7, $8, $8, \
             'staff', $8, 'COMPLETED', $9::uuid)",
    )
    .bind(tx_id)
    .bind(doc_ref)
    .bind(branch)
    .bind(shift)
    .bind(sub_id)
    .bind(customer_id)
    .bind(date)
    .bind(recorded_at)
    .bind(org)
    .execute(pool)
    .await
    .expect("insert transaction");

    sqlx::query(
        "INSERT INTO transaction_lines ( \
             id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
             denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
             thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
             line_no, created_at, updated_at \
         ) VALUES ( \
             $1, $2::uuid, $3, NULL, 'SELL', $4, \
             NULL, 100.0, 1.0, 1.0, NULL, \
             $5::numeric, 0.00, $5::numeric, $6::numeric, $7::numeric, \
             1, $8, $8 \
         )",
    )
    .bind(line_id)
    .bind(org)
    .bind(tx_id)
    .bind(currency)
    .bind(thb_amount)
    .bind(cost_basis_thb)
    .bind(realized_margin_thb)
    .bind(recorded_at)
    .execute(pool)
    .await
    .expect("insert transaction_line");

    line_id.to_string()
}

async fn buy_line(
    pool: &PgPool,
    org: &str,
    branch: &str,
    date: NaiveDate,
    currency: &str,
    thb_amount: &str,
) -> String {
    let tx_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    let sub_id = unique("sub");
    let doc_ref = unique("TX");
    let recorded_at = date.and_hms_opt(10, 0, 0).unwrap().and_utc();

    let shift = format!("shift-{branch}");
    let customer_id = format!("cust-{branch}");

    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
             customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
             recorded_by, completed_at, status, organization_id) VALUES ($1, $2, $3, 'LIVE', $4, $5, 'hash', $6, 'Customer', 'THA', 'staff', '', '', $7, $8, $8, \
             'staff', $8, 'COMPLETED', $9::uuid)",
    )
    .bind(tx_id)
    .bind(doc_ref)
    .bind(branch)
    .bind(shift)
    .bind(sub_id)
    .bind(customer_id)
    .bind(date)
    .bind(recorded_at)
    .bind(org)
    .execute(pool)
    .await
    .expect("insert transaction");

    sqlx::query(
        "INSERT INTO transaction_lines ( \
             id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
             denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
             thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
             line_no, created_at, updated_at \
         ) VALUES ( \
             $1, $2::uuid, $3, NULL, 'BUY', $4, \
             NULL, 100.0, 1.0, 1.0, NULL, \
             $5::numeric, 0.00, $5::numeric, NULL, NULL, \
             1, $6, $6 \
         )",
    )
    .bind(line_id)
    .bind(org)
    .bind(tx_id)
    .bind(currency)
    .bind(thb_amount)
    .bind(recorded_at)
    .execute(pool)
    .await
    .expect("insert transaction_line");

    line_id.to_string()
}

async fn insert_101_uncosted_lines(pool: &PgPool, org: &str, branch: &str, date: NaiveDate) {
    let recorded_at = date.and_hms_opt(10, 0, 0).unwrap().and_utc();
    let shift = format!("shift-{branch}");
    sqlx::query(
        "INSERT INTO transactions ( \
             id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by, completed_at, status, organization_id \
         ) \
         SELECT \
             gen_random_uuid(), 'TX-CAP-' || n, $1, 'LIVE', $2, 'sub-cap-' || n, 'hash', \
             'cust-' || $1, 'Customer', 'THA', 'staff', '', '', \
             $3, $4, $4, 'staff', $4, 'COMPLETED', $5::uuid \
         FROM generate_series(1, 101) AS n",
    )
    .bind(branch)
    .bind(shift)
    .bind(date)
    .bind(recorded_at)
    .bind(org)
    .execute(pool)
    .await
    .expect("insert 101 transactions");

    sqlx::query(
        "INSERT INTO transaction_lines ( \
             id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
             denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
             thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
             line_no, created_at, updated_at \
         ) \
         SELECT \
             gen_random_uuid(), t.organization_id, t.id, NULL, 'SELL', 'USD', \
             NULL, 100.0, 1.0, 1.0, NULL, \
             100.00, 0.00, 100.00, NULL, NULL, \
             1, $1, $1 \
         FROM transactions t WHERE t.submission_id LIKE 'sub-cap-%'",
    )
    .bind(recorded_at)
    .execute(pool)
    .await
    .expect("insert 101 transaction_lines");
}

#[tokio::test]
async fn a_clean_sell_margin_equals_sales_minus_cost_has_no_offender() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // Line 1: sales 3200, cost 3000, margin 200
    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("200.00"),
    )
    .await;
    // Line 2: sales 1850, cost 1800, margin 50
    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "EUR",
        "1850.00",
        Some("1800.00"),
        Some("50.00"),
    )
    .await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(found, vec![]);

    pool.close().await;
}

#[tokio::test]
async fn a_sell_line_whose_realized_margin_differs_from_sales_minus_cost_is_reported() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // Sales 3200, cost 3000 (diff 200), but margin is hand-edited to 210
    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("210.00"),
    )
    .await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(keys(&found), ["B1:2026-10-06"]);
    assert!(!found[0].detail.is_empty(), "detail describes the sums");

    pool.close().await;
}

#[tokio::test]
async fn a_margin_off_by_one_satang_is_reported() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // Expected margin = 200.00; hand-edited to 200.01 (+0.01 satang).
    // Catches mutants that allow loose tolerance.
    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("200.01"),
    )
    .await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(keys(&found), ["B1:2026-10-06"]);

    pool.close().await;
}

#[tokio::test]
async fn a_sell_line_with_null_cost_basis_is_reported_by_line_id_as_uncosted() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    let line_id = sell_line(&pool, ORG, "B1", d, "USD", "3200.00", None, None).await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(keys(&found), [line_id]);
    assert_eq!(found[0].detail, "uncosted");

    pool.close().await;
}

#[tokio::test]
async fn uncosted_line_is_reported_and_not_summed_with_clean_lines() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // Clean line: sales 3200, cost 3000, margin 200
    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("200.00"),
    )
    .await;
    // Uncosted line: cost is NULL
    let uncosted_id = sell_line(&pool, ORG, "B1", d, "EUR", "1850.00", None, None).await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    // Only the uncosted line is reported by its id; clean line does not produce a branch:period offender
    assert_eq!(keys(&found), [uncosted_id]);
    assert_eq!(found[0].detail, "uncosted");

    pool.close().await;
}

#[tokio::test]
async fn the_period_filter_excludes_a_broken_line_outside_the_period() {
    let pool = db().await;
    let d1 = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // Broken line on 2026-10-05
    sell_line(
        &pool,
        ORG,
        "B1",
        d1,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("250.00"),
    )
    .await;

    // Scope for 2026-10-06 only: must exclude 2026-10-05
    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d2, d2)))).await;
    assert_eq!(found, vec![]);

    pool.close().await;
}

#[tokio::test]
async fn no_period_in_scope_checks_every_line_of_the_scope() {
    let pool = db().await;
    let d1 = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();

    // Broken line on 2026-10-05
    sell_line(
        &pool,
        ORG,
        "B1",
        d1,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("250.00"),
    )
    .await;

    // Scope without period: checks all lines of B1
    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), None)).await;
    assert_eq!(keys(&found), ["B1:2026-10-05"]);

    pool.close().await;
}

#[tokio::test]
async fn buy_lines_without_margins_are_ignored() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // BUY line has no cost_basis_thb or realized_margin_thb; must not be flagged
    buy_line(&pool, ORG, "B1", d, "USD", "3100.00").await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(found, vec![]);

    pool.close().await;
}

#[tokio::test]
async fn the_scope_filters_by_branch_and_by_organisation() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // Clean line in B1
    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("200.00"),
    )
    .await;
    // Broken line in B2
    sell_line(
        &pool,
        ORG,
        "B2",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("999.00"),
    )
    .await;
    // Broken line in X1 (different org)
    sell_line(
        &pool,
        OTHER_ORG,
        "X1",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("999.00"),
    )
    .await;

    // Scope for B1: clean
    let found_b1 = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(found_b1, vec![]);

    // Scope for entire ORG (branch None): only B2 is reported, X1 of OTHER_ORG is ignored
    let found_org = offenders(&pool, &MarginCheck, &scope(None, Some((d, d)))).await;
    assert_eq!(keys(&found_org), ["B2:2026-10-06"]);

    pool.close().await;
}

#[tokio::test]
async fn more_than_100_offenders_give_100_and_the_marker() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    insert_101_uncosted_lines(&pool, ORG, "B1", d).await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_capped(&found);

    pool.close().await;
}

#[tokio::test]
async fn query_is_one_capped_statement_and_the_check_is_registered() {
    assert!(QUERY.ends_with("LIMIT 101"));
    let pool = db().await;
    assert_registered(&pool, 4).await;
    assert_eq!(MarginCheck.id(), 4);
    pool.close().await;
}

// ------------------------------------------------------------ the lane controller's ruling issuecomment-6107257494

/// T1: since #924 a SELL below cost has a negative margin (a signed `MoneyDelta`). Sales 300.00, cost 320.00
/// and margin −20.00 balance, so a check that compares absolute values must not report it.
#[tokio::test]
async fn a_sell_below_cost_with_its_negative_margin_has_no_offender() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "300.00",
        Some("320.00"),
        Some("-20.00"),
    )
    .await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(found, vec![]);

    pool.close().await;
}

/// T1's broken twin: sales 300.00 and cost 320.00 need margin −20.00; −19.00 is reported.
#[tokio::test]
async fn a_sell_below_cost_whose_negative_margin_is_off_is_reported() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "300.00",
        Some("320.00"),
        Some("-19.00"),
    )
    .await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(keys(&found), ["B1:2026-10-06"]);

    pool.close().await;
}

/// The sign matters: a SELL below cost booked with margin +20.00 instead of −20.00 is reported. A check that
/// compares absolute values on both sides (|20| = |300 − 320|) would pass it.
#[tokio::test]
async fn a_sell_below_cost_booked_with_a_positive_margin_is_reported() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "300.00",
        Some("320.00"),
        Some("20.00"),
    )
    .await;

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(keys(&found), ["B1:2026-10-06"]);

    pool.close().await;
}

/// The day in the key and in the period is `transactions.value_date` (the business date), not the UTC day of
/// `recorded_at` or of the line's `created_at`, nor their day in the organisation's timezone (Asia/Bangkok). The line
/// below is booked on 2026-10-05 but recorded at 2026-10-04T10:00Z, which is 17:00 on 2026-10-04 in Bangkok.
#[tokio::test]
async fn the_day_of_the_key_and_the_period_is_the_value_date() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let the_day_before = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();

    let line = sell_line(
        &pool,
        ORG,
        "B1",
        d,
        "USD",
        "3200.00",
        Some("3000.00"),
        Some("250.00"),
    )
    .await;
    let recorded = "2026-10-04T10:00:00Z";
    sqlx::query(
        "UPDATE transactions SET recorded_at = $2::timestamptz, paid_at = $2::timestamptz, \
         completed_at = $2::timestamptz \
         WHERE id = (SELECT transaction_id FROM transaction_lines WHERE id = $1::uuid)",
    )
    .bind(&line)
    .bind(recorded)
    .execute(&pool)
    .await
    .expect("record the transaction on the previous UTC day");
    sqlx::query(
        "UPDATE transaction_lines SET created_at = $2::timestamptz, updated_at = $2::timestamptz \
         WHERE id = $1::uuid",
    )
    .bind(&line)
    .bind(recorded)
    .execute(&pool)
    .await
    .expect("the line too");

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), None)).await;
    assert_eq!(
        keys(&found),
        ["B1:2026-10-05"],
        "the key's day is value_date"
    );

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(
        keys(&found),
        ["B1:2026-10-05"],
        "a period of value_date's day includes the line"
    );

    let found = offenders(
        &pool,
        &MarginCheck,
        &scope(Some("B1"), Some((the_day_before, the_day_before))),
    )
    .await;
    assert_eq!(
        found,
        vec![],
        "the UTC day of recorded_at is not the period's day"
    );

    pool.close().await;
}

#[tokio::test]
async fn non_completed_sell_lines_are_ignored() {
    let pool = db().await;
    let d = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();

    // 1. CONFIRMED SELL line with NULL cost_basis_thb
    let tx_id1 = Uuid::new_v4();
    let line_id1 = Uuid::new_v4();
    let sub_id1 = unique("sub");
    let recorded_at = d.and_hms_opt(10, 0, 0).unwrap().and_utc();
    let shift = "shift-B1";
    let customer_id = "cust-B1";

    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
             customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
             recorded_by, completed_at, status, organization_id) VALUES ($1, NULL, 'B1', 'LIVE', $2, $3, 'hash', $4, 'Customer', 'THA', 'staff', '', '', $5, $6, \
             $6, 'staff', NULL, 'CONFIRMED', $7::uuid)",
    )
    .bind(tx_id1)
    .bind(shift)
    .bind(sub_id1)
    .bind(customer_id)
    .bind(d)
    .bind(recorded_at)
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("insert confirmed transaction");

    sqlx::query(
        "INSERT INTO transaction_lines ( \
             id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
             denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
             thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
             line_no, created_at, updated_at \
         ) VALUES ( \
             $1, $2::uuid, $3, NULL, 'SELL', 'USD', \
             NULL, 100.0, 1.0, 1.0, NULL, \
             3200.00, 0.00, 3200.00, NULL, NULL, \
             1, $4, $4 \
         )",
    )
    .bind(line_id1)
    .bind(ORG)
    .bind(tx_id1)
    .bind(recorded_at)
    .execute(&pool)
    .await
    .expect("insert confirmed line");

    // 2. CANCELLED SELL line whose margin is wrong (sales 3200, cost 3000, margin 999)
    let tx_id2 = Uuid::new_v4();
    let line_id2 = Uuid::new_v4();
    let sub_id2 = unique("sub");

    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
             customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
             recorded_by, completed_at, status, organization_id, cancelled_at, cancel_reason) VALUES ($1, NULL, 'B1', 'LIVE', $2, $3, 'hash', $4, 'Customer', 'THA', 'staff', '', '', $5, $6, \
             $6, 'staff', NULL, 'CANCELLED', $7::uuid, $6, 'Cancelled by customer')",
    )
    .bind(tx_id2)
    .bind(shift)
    .bind(sub_id2)
    .bind(customer_id)
    .bind(d)
    .bind(recorded_at)
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("insert cancelled transaction");

    sqlx::query(
        "INSERT INTO transaction_lines ( \
             id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
             denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
             thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
             line_no, created_at, updated_at \
         ) VALUES ( \
             $1, $2::uuid, $3, NULL, 'SELL', 'USD', \
             NULL, 100.0, 1.0, 1.0, NULL, \
             3200.00, 0.00, 3200.00, 3000.00, 999.00, \
             1, $4, $4 \
         )",
    )
    .bind(line_id2)
    .bind(ORG)
    .bind(tx_id2)
    .bind(recorded_at)
    .execute(&pool)
    .await
    .expect("insert cancelled line");

    let found = offenders(&pool, &MarginCheck, &scope(Some("B1"), Some((d, d)))).await;
    assert_eq!(
        found,
        vec![],
        "non-COMPLETED sell lines must produce no offenders"
    );

    pool.close().await;
}
