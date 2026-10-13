//! SP-2 (#1583, rates.md §12.6, §12.10) — ReferenceBase fallback to starting prices.

mod rate_engine_common;

use chrono::{Duration, Utc};
use rate_engine_common::TestDb;
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::draft::reference_base::ReferenceBase;

const ORG_1: Uuid = Uuid::from_u128(1);
const ORG_2: Uuid = Uuid::from_u128(2);

async fn seed_org_2(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1, 'ORG2', 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(ORG_2)
    .execute(pool)
    .await
    .expect("insert org 2");
}

async fn seed_branch(pool: &PgPool, id: &str, org_id: Uuid) {
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id) \
         VALUES ($1, $1, $2) \
         ON CONFLICT (id) DO UPDATE SET organization_id = EXCLUDED.organization_id",
    )
    .bind(id)
    .bind(org_id)
    .execute(pool)
    .await
    .expect("insert branch");
}

async fn seed_batch_and_line(
    pool: &PgPool,
    line_id: Uuid,
    currency: &str,
    ref_type: &str,
    provider: &str,
    fetched_at: chrono::DateTime<Utc>,
) {
    let batch_id = Uuid::new_v4();
    let checksum = format!("chk-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES ($1, $2, CURRENT_DATE, $3, $4, 'COMPLETE')",
    )
    .bind(batch_id)
    .bind(provider)
    .bind(fetched_at)
    .bind(checksum)
    .execute(pool)
    .await
    .expect("insert rate_reference_batches");

    sqlx::query(
        "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES ($1, $2, $3, $4, 1.0, 35.0, 36.0, '{}'::jsonb)",
    )
    .bind(line_id)
    .bind(batch_id)
    .bind(currency)
    .bind(ref_type)
    .execute(pool)
    .await
    .expect("insert rate_reference_lines");
}

/// Row 1: a 'BOT' line exists for the currency (and a 'START' too) -> the newest 'BOT' line.
#[tokio::test]
async fn test_row1_bot_line_exists_and_start_too_returns_newest_bot_line() {
    let db = TestDb::start().await;
    let branch = BranchId::new("B1");
    seed_branch(&db.pool, branch.as_str(), ORG_1).await;

    let now = Utc::now();
    let start_line_id = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        start_line_id,
        "USD",
        "START",
        &format!("START:{ORG_1}"),
        now - Duration::hours(3),
    )
    .await;

    let bot_line_old = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        bot_line_old,
        "USD",
        "BOT",
        "BOT",
        now - Duration::hours(2),
    )
    .await;

    let bot_line_new = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        bot_line_new,
        "USD",
        "BOT",
        "BOT",
        now - Duration::hours(1),
    )
    .await;

    let usd = CurrencyCode::parse("USD").unwrap();
    let mut tx = db.begin().await;
    let result = ReferenceBase::newest_in(&mut tx, &branch, &usd)
        .await
        .expect("query succeeds");

    assert_eq!(
        result,
        Some(bot_line_new),
        "Row 1: when BOT line exists (and START too), newest BOT line must be returned"
    );
}

/// Row 2: no 'BOT' line ever for the currency, own org has 'START' lines -> the org's newest 'START' line.
#[tokio::test]
async fn test_row2_no_bot_line_ever_own_org_has_start_lines_returns_newest_start_line() {
    let db = TestDb::start().await;
    let branch = BranchId::new("B1");
    seed_branch(&db.pool, branch.as_str(), ORG_1).await;

    let now = Utc::now();
    let start_line_old = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        start_line_old,
        "USD",
        "START",
        &format!("START:{ORG_1}"),
        now - Duration::hours(2),
    )
    .await;

    let start_line_new = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        start_line_new,
        "USD",
        "START",
        &format!("START:{ORG_1}"),
        now - Duration::hours(1),
    )
    .await;

    let usd = CurrencyCode::parse("USD").unwrap();
    let mut tx = db.begin().await;
    let result = ReferenceBase::newest_in(&mut tx, &branch, &usd)
        .await
        .expect("query succeeds");

    assert_eq!(
        result,
        Some(start_line_new),
        "Row 2: no BOT line ever, own org has START lines -> returns org's newest START line"
    );
}

/// Row 3: no 'BOT' ever, only another organisation has a 'START' line -> None.
#[tokio::test]
async fn test_row3_no_bot_ever_only_another_org_has_start_line_returns_none() {
    let db = TestDb::start().await;
    seed_org_2(&db.pool).await;

    let branch1 = BranchId::new("B1");
    seed_branch(&db.pool, branch1.as_str(), ORG_1).await;

    let now = Utc::now();
    let start_line_org2 = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        start_line_org2,
        "USD",
        "START",
        &format!("START:{ORG_2}"),
        now - Duration::hours(1),
    )
    .await;

    let usd = CurrencyCode::parse("USD").unwrap();
    let mut tx = db.begin().await;
    let result = ReferenceBase::newest_in(&mut tx, &branch1, &usd)
        .await
        .expect("query succeeds");

    assert_eq!(
        result, None,
        "Row 3: no BOT ever, only another org has START line -> returns None"
    );
}

/// Row 4: a 'BOT' line written after a 'START' line -> the 'BOT' line, and 'START' is never used again for that currency.
#[tokio::test]
async fn test_row4_bot_line_written_after_start_line_returns_bot_and_start_never_used_again() {
    let db = TestDb::start().await;
    let branch = BranchId::new("B1");
    seed_branch(&db.pool, branch.as_str(), ORG_1).await;

    let now = Utc::now();
    let start_line_1 = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        start_line_1,
        "USD",
        "START",
        &format!("START:{ORG_1}"),
        now - Duration::hours(3),
    )
    .await;

    let bot_line = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        bot_line,
        "USD",
        "BOT",
        "BOT",
        now - Duration::hours(2),
    )
    .await;

    // Even if a subsequent START line is added later, BOT line is still chosen
    let start_line_2 = Uuid::new_v4();
    seed_batch_and_line(
        &db.pool,
        start_line_2,
        "USD",
        "START",
        &format!("START:{ORG_1}"),
        now - Duration::hours(1),
    )
    .await;

    let usd = CurrencyCode::parse("USD").unwrap();
    let mut tx = db.begin().await;
    let result = ReferenceBase::newest_in(&mut tx, &branch, &usd)
        .await
        .expect("query succeeds");

    assert_eq!(
        result,
        Some(bot_line),
        "Row 4: BOT line written after START line -> returns BOT line, START never used again"
    );
}

/// Row 5: neither exists -> None.
#[tokio::test]
async fn test_row5_neither_exists_returns_none() {
    let db = TestDb::start().await;
    let branch = BranchId::new("B1");
    seed_branch(&db.pool, branch.as_str(), ORG_1).await;

    let usd = CurrencyCode::parse("USD").unwrap();
    let mut tx = db.begin().await;
    let result = ReferenceBase::newest_in(&mut tx, &branch, &usd)
        .await
        .expect("query succeeds");

    assert_eq!(
        result, None,
        "Row 5: neither BOT nor START line exists -> returns None"
    );
}
