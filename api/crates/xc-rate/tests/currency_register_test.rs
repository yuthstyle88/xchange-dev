//! Ticket #595 (RU-1) — the currency register's `unit_size` and `minor_unit`, and migration 1's shape.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §3.1, §8 migration 1 (as corrected: **no stored rate is
//! converted in migration 1**; that moves to RE-4's migration 5). Owner A1 (2026-10-09): the
//! Blueprint's units, JPY 100, KRW 1000, every other currency 1. Minor units from ISO 4217.
//!
//! Needs a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`).
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f): RE-7 drops `rates`
//! (`docs/design/blueprint/rates.md` §8 migration 6). `migration_one_widens_and_converts_nothing`
//! is about what migration 1 did to `rates`, so it now applies only the migrations older than the
//! drop (`TestDb::start_before_rates_drop`) and still writes and reads its per-1 JPY row there;
//! that stays true after RE-7. Nothing it checks changed.

mod common;

use std::str::FromStr;

use common::*;
use rust_decimal::Decimal;
use xc_kernel::money::CurrencyCode;
use xc_rate::domain::currency::currency_register::CurrencyRegister;
use xc_rate::infrastructure::currency::currency_register::PgCurrencyRegister;

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known code")
}

#[tokio::test]
async fn the_register_answers_unit_size_and_minor_unit() {
    let db = TestDb::start().await;
    let register = PgCurrencyRegister::new(db.pool.clone());
    for (c, unit, minor) in [
        ("JPY", "100", 0u8),
        ("KRW", "1000", 0),
        ("USD", "1", 2),
        ("EUR", "1", 2),
        ("THB", "1", 2),
        ("BHD", "1", 3),
        ("KWD", "1", 3),
    ] {
        let u = register
            .units(code(c))
            .await
            .unwrap_or_else(|e| panic!("{c}: {e:?}"))
            .unwrap_or_else(|| panic!("{c}: in the register"));
        assert_eq!(u.unit_size().value(), d(unit), "{c}: unit_size");
        assert_eq!(u.minor_unit().get(), minor, "{c}: minor_unit");
    }
    db.finish().await;
}

#[tokio::test]
async fn migration_one_widens_and_converts_nothing() {
    // RE-7 #604: `rates` exists only before migration 6, which is not applied here.
    let db = TestDb::start_before_rates_drop().await;
    // every rate column is numeric(20,8), and the unit_size columns are numeric(20,4). The amount and
    // till columns are not widened here: their readers still decode them at 2 places, so each is widened
    // by the ticket that moves its reader (design rates.md §8 migration 1, as corrected for #595)
    for (table, column, precision, scale) in [
        ("rates", "buy", 20, 8),
        ("rates", "sell", 20, 8),
        ("rates", "partner_cost", 20, 8),
        ("transactions", "rate", 20, 8),
        ("transactions", "reference_rate", 20, 8),
        ("kiosk_requests", "rate", 20, 8),
        ("bookings", "rate", 20, 8),
        ("currencies", "unit_size", 20, 4),
        ("transactions", "unit_size", 20, 4),
        ("kiosk_requests", "unit_size", 20, 4),
        ("bookings", "unit_size", 20, 4),
    ] {
        let (p, s): (i32, i32) = sqlx::query_as(
            "SELECT numeric_precision, numeric_scale FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{table}.{column}: {e}"));
        assert_eq!((p, s), (precision, scale), "{table}.{column}");
    }

    // no trigger or default rewrites a per-1 JPY rate: until RE-4 the readers still compute
    // amount × rate, so a converted rate would make every quote 100× too high (the conversion is
    // migration 5's, with RE-4's readers)
    db.seed_branch("B1", "Branch one").await;
    sqlx::query(
        "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
         VALUES ('B1', 'JPY', 0.2300, 0.2350, now(), 'test', 'ru1-jpy-request-0001')",
    )
    .execute(&db.pool)
    .await
    .expect("a per-1 JPY rate");
    let (buy, sell): (Decimal, Decimal) =
        sqlx::query_as("SELECT buy, sell FROM rates WHERE branch_id = 'B1' AND code = 'JPY'")
            .fetch_one(&db.pool)
            .await
            .expect("read back");
    assert_eq!(
        (buy, sell),
        (d("0.23"), d("0.235")),
        "stored per 1, unchanged"
    );
    db.finish().await;
}
