//! Ticket #598 (RE-1+RE-3) — migration 4 (`20261010120000_rate_engine.sql`): its backfill builds
//! publication version 1 of every branch from the newest `rates` row per currency (RE-1).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §8 migration 4 (corrected for #595): for each branch
//! with `rates` rows, publication version 1 from the newest row per (branch, code); lines MANUAL,
//! `buy_rate`/`sell_rate` = the per-1 value **× `currencies.unit_size`**, `unit_size` copied from
//! `currencies`; `source_snapshot = {"migrated_from": "rates", "rates_id": …, …}`;
//! `published_by` = the row's `updated_by`; one `RATE_PUBLICATION` request per branch with a
//! decision `{approver_id: "migration", reason: "migrated_from_rates"}`; the branch pointer set to
//! it; branches with no rates get no publication; `rates` is not dropped. §4.1 (tables), §9 RE-1.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`); nothing here skips. The test
//! applies every migration older than migration 4, writes `rates` rows as the direct save writes
//! them (per 1, `numeric(20,8)` since migration 1), then applies the rest.
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f): RE-7's migration 6
//! (`<version>_drop_rates.sql`, `docs/design/blueprint/rates.md` §8) drops `rates`. This test is
//! about what migration 4 did to `rates`, so "the rest" now stops before migration 6: the rows are
//! still written with `INSERT INTO rates` and read back after migration 4, which stays correct
//! after RE-7. Nothing it checks changed.
//!
//! Assumptions:
//! - "Newest" is the highest `rates.id` per (branch, code), as `rates_branch_code_id_idx` orders
//!   and `RateService::current` reads; every newer row here also has a later `updated_at`, so the
//!   two orders agree.
//! - The request's `approval_requests.branch_id` is the branch; `rate_publications
//!   .approved_request_id` is that request; `published_rate_lines.denomination_group` is `'ALL'`.
//! - `source_snapshot->>'rates_id'` is the source row's id as text (a JSON number or string).

mod rate_engine_common;

use std::borrow::Cow;
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;

const MIGRATION_4: i64 = 20261010120000;
/// The description sqlx gives RE-7's migration 6, which drops `rates` (RE-7 #604).
const DROP_RATES: &str = "drop rates";

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

/// 2026-10-08 00:00 UTC plus `minutes`.
fn at(minutes: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_417_600 + minutes * 60, 0)
        .single()
        .expect("a valid instant")
}

async fn migrator() -> Migrator {
    let dir = migrations_dir();
    Migrator::new(dir.as_path())
        .await
        .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
}

/// A row as the direct save writes it; answers its id.
async fn save(
    db: &TestDb,
    branch: &str,
    code: &str,
    buy: &str,
    sell: &str,
    when: DateTime<Utc>,
    by: &str,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
         VALUES ($1, $2, $3::numeric, $4::numeric, $5, $6, $7) RETURNING id",
    )
    .bind(branch)
    .bind(code)
    .bind(buy)
    .bind(sell)
    .bind(when)
    .bind(by)
    .bind(unique("req"))
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("insert into rates ({branch} {code}): {e}"))
}

/// `(currency_code, buy_rate, sell_rate, unit_size, denomination_group, rates_id)` of every line
/// of the branch's current publication, by currency.
async fn current_lines(
    db: &TestDb,
    branch: &str,
) -> Vec<(
    String,
    Option<Decimal>,
    Option<Decimal>,
    Decimal,
    String,
    String,
)> {
    sqlx::query_as(
        "SELECT trim(l.currency_code), l.buy_rate, l.sell_rate, l.unit_size, l.denomination_group, \
                l.source_snapshot->>'rates_id' \
         FROM branches b JOIN published_rate_lines l ON l.publication_id = b.current_publication_id \
         WHERE b.id = $1 AND l.source_snapshot->>'migrated_from' = 'rates' \
         ORDER BY l.currency_code",
    )
    .bind(branch)
    .fetch_all(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("lines of {branch}: {e}"))
}

#[tokio::test]
async fn the_backfill_builds_version_one_from_the_newest_row_per_code_times_unit_size() {
    let db = TestDb::empty().await;
    let all = migrator().await;
    let mut before = migrator().await;
    before.migrations = Cow::Owned(
        all.migrations
            .iter()
            .filter(|m| m.version < MIGRATION_4)
            .cloned()
            .collect(),
    );
    assert!(
        all.migrations.iter().any(|m| m.version == MIGRATION_4),
        "api/migrations has migration 4, 20261010120000_rate_engine.sql"
    );
    before
        .run(&db.pool)
        .await
        .expect("apply the migrations before migration 4");

    db.seed_branch("XC-B02").await;
    db.seed_branch("XC-B03").await;
    // MAIN: USD twice (the newer wins), JPY and KRW per 1 (× 100 and × 1000 on the way in)
    save(&db, MAIN, "USD", "32", "32.3", at(0), "owner").await;
    let usd = save(&db, MAIN, "USD", "32.1", "32.45", at(10), "owner").await;
    let jpy_old = save(&db, MAIN, "JPY", "0.2201", "0.225", at(1), "owner").await;
    let jpy = save(&db, MAIN, "JPY", "0.2301", "0.235", at(11), "owner").await;
    let krw = save(&db, MAIN, "KRW", "0.0251", "0.02612345", at(12), "owner").await;
    // XC-B02: one EUR row, saved by a RATES employee
    let eur = save(&db, "XC-B02", "EUR", "37.4", "38", at(5), "rates-9").await;
    // XC-B03: no rates
    let rates_before = db.count("rates").await;
    assert!(jpy_old < jpy);

    // RE-7 #604: every migration up to, not including, the one that drops `rates`.
    let drop = all
        .migrations
        .iter()
        .find(|m| m.description == DROP_RATES)
        .map_or(i64::MAX, |m| m.version);
    let mut after = migrator().await;
    after.migrations = Cow::Owned(
        all.migrations
            .iter()
            .filter(|m| m.version < drop)
            .cloned()
            .collect(),
    );
    after
        .run(&db.pool)
        .await
        .expect("apply migration 4 and the rest, up to the drop of rates");

    // MAIN: version 1 with the newest row of each code, × unit_size
    assert_eq!(
        current_lines(&db, MAIN).await,
        vec![
            (
                "JPY".to_owned(),
                Some(d("23.01")),
                Some(d("23.5")),
                d("100"),
                "ALL".to_owned(),
                jpy.to_string()
            ),
            (
                "KRW".to_owned(),
                Some(d("25.1")),
                Some(d("26.12345")),
                d("1000"),
                "ALL".to_owned(),
                krw.to_string()
            ),
            (
                "USD".to_owned(),
                Some(d("32.1")),
                Some(d("32.45")),
                d("1"),
                "ALL".to_owned(),
                usd.to_string()
            ),
        ]
    );
    assert_eq!(
        current_lines(&db, "XC-B02").await,
        vec![(
            "EUR".to_owned(),
            Some(d("37.4")),
            Some(d("38")),
            d("1"),
            "ALL".to_owned(),
            eur.to_string()
        )]
    );

    // one publication per branch with rates, version 1, published by the row's updated_by,
    // approved by one RATE_PUBLICATION request the migration decided
    for (branch, by) in [(MAIN, "owner"), ("XC-B02", "rates-9")] {
        let rows: Vec<(i32, String, String, String, String)> = sqlx::query_as(
            "SELECT p.version, p.published_by, r.type, r.status, coalesce(r.branch_id, '') \
             FROM rate_publications p JOIN approval_requests r ON r.id = p.approved_request_id \
             WHERE p.branch_id = $1",
        )
        .bind(branch)
        .fetch_all(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("publications of {branch}: {e}"));
        assert_eq!(
            rows,
            vec![(
                1,
                by.to_owned(),
                "RATE_PUBLICATION".to_owned(),
                "APPROVED".to_owned(),
                branch.to_owned()
            )],
            "{branch}"
        );
        let decisions: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT d.approver_id, d.outcome, d.reason \
             FROM rate_publications p JOIN approval_decisions d \
               ON d.request_id = p.approved_request_id \
             WHERE p.branch_id = $1",
        )
        .bind(branch)
        .fetch_all(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("decisions of {branch}: {e}"));
        assert_eq!(
            decisions,
            vec![(
                "migration".to_owned(),
                "APPROVED".to_owned(),
                "migrated_from_rates".to_owned()
            )],
            "{branch}"
        );
        let pointed: bool = sqlx::query_scalar(
            "SELECT b.current_publication_id = p.id FROM branches b \
             JOIN rate_publications p ON p.branch_id = b.id WHERE b.id = $1",
        )
        .bind(branch)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("pointer of {branch}: {e}"));
        assert!(pointed, "{branch}: the pointer is set to version 1");
    }

    // nothing for a branch without rates, and no line for a superseded row
    assert_eq!(db.current_publication("XC-B03").await, None);
    assert_eq!(db.count("rate_publications").await, 2);
    assert_eq!(db.count("published_rate_lines").await, 4);
    assert_eq!(db.count("approval_requests").await, 2);
    // `rates` is not dropped or rewritten here (RE-7 drops it)
    assert_eq!(db.count("rates").await, rates_before);
    let jpy_row: (Decimal, Decimal) = sqlx::query_as("SELECT buy, sell FROM rates WHERE id = $1")
        .bind(jpy)
        .fetch_one(&db.pool)
        .await
        .expect("the JPY row");
    assert_eq!(
        jpy_row,
        (d("0.2301"), d("0.235")),
        "the per-1 row is not converted"
    );
    db.finish().await;
}

#[tokio::test]
async fn on_an_empty_database_the_migration_publishes_nothing() {
    let db = TestDb::start().await;
    assert_eq!(db.count("rate_publications").await, 0);
    assert_eq!(db.count("published_rate_lines").await, 0);
    assert_eq!(db.count("approval_requests").await, 0);
    assert_eq!(db.current_publication(MAIN).await, None);
    db.finish().await;
}
