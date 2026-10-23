//! P-1 up-front tests (#2054, design `docs/design/blueprint/bounded-inputs.md` §2 B-3, B-7, §3 `:72-78`) — written
//! before the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller
//! claude/56a74fcd. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `v3::input` is the one place a v3 command turns a refused input into the house answer: **400
//! `validationFailed`** naming the field. It never trims (open point O-P1 on the ticket: these tests pin "no trim",
//! so v3 answers exactly as today's `parse_decimal`).
//!
//! `must_exist` needs `TEST_DATABASE_URL` (a PostgreSQL database whose name ends in `_test`); nothing skips.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use actix_web::http::StatusCode;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::money::{Bound, IntBound, Sign};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::http::v3::input::{self, RowRef};

const N20_4_POSITIVE: Bound = Bound::numeric(20, 4, Sign::Positive);

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("test decimal")
}

fn refused(r: Result<impl std::fmt::Debug, V3Error>, field: &str, what: &str) {
    match r {
        Ok(v) => panic!("{what}: must be refused, got Ok({v:?})"),
        Err(e) => {
            assert_eq!(e.status, StatusCode::BAD_REQUEST, "{what}: {e:?}");
            assert_eq!(e.code, "validationFailed", "{what}: {e:?}");
            assert_eq!(e.field, Some(field), "{what}: {e:?}");
            assert!(!e.message.is_empty(), "{what}: a message");
        }
    }
}

// ---------------------------------------------------------------- decimal

#[test]
fn decimal_returns_the_bounded_value() {
    assert_eq!(
        input::decimal("quantity", "12.5", N20_4_POSITIVE).expect("12.5"),
        d("12.5")
    );
    assert_eq!(
        input::decimal("quantity", "9999999999999999.9999", N20_4_POSITIVE).expect("max"),
        d("9999999999999999.9999")
    );
}

#[test]
fn every_bound_refusal_is_400_validation_failed_on_the_field() {
    for (text, what) in [
        ("", "empty"),
        ("abc", "syntax"),
        ("1e5", "exponent"),
        ("0.00001", "scale"),
        ("0", "sign: zero under Positive"),
        ("-1", "sign: negative"),
        ("10000000000000000", "range: 10^16"),
        (
            "99999999999999999999999999999999999999",
            "range: past Decimal",
        ),
    ] {
        refused(
            input::decimal("quantity", text, N20_4_POSITIVE),
            "quantity",
            what,
        );
    }
}

#[test]
fn decimal_does_not_trim_surrounding_whitespace() {
    // O-P1: one rule for every v3 route. Not trimming keeps v3 identical to today's `parse_decimal`.
    for text in [" 1", "1 ", "\t1", "1\n"] {
        refused(
            input::decimal("amount", text, N20_4_POSITIVE),
            "amount",
            &format!("{text:?}"),
        );
    }
}

#[test]
fn decimal_names_the_field_it_was_given() {
    refused(
        input::decimal("lines.quantity", "x", N20_4_POSITIVE),
        "lines.quantity",
        "nested field name",
    );
}

// ---------------------------------------------------------------- one_of

#[test]
fn one_of_returns_the_matching_allowed_value() {
    let allowed: &[&'static str] = &["CUPS", "TCP"];
    assert_eq!(
        input::one_of("transport", "CUPS", allowed).expect("CUPS"),
        "CUPS"
    );
    assert_eq!(
        input::one_of("transport", "TCP", allowed).expect("TCP"),
        "TCP"
    );
}

#[test]
fn one_of_refuses_anything_else_exactly() {
    let allowed: &[&'static str] = &["CUPS", "TCP"];
    for text in ["", "cups", "Tcp", " TCP", "TCP ", "USB"] {
        refused(
            input::one_of("transport", text, allowed),
            "transport",
            &format!("{text:?}"),
        );
    }
}

// ---------------------------------------------------------------- integer

#[test]
fn integer_accepts_both_ends_and_refuses_one_past_each() {
    let b = IntBound::integer(5, 240);
    assert_eq!(input::integer("slotMinutes", 5, b).expect("5"), 5);
    assert_eq!(input::integer("slotMinutes", 240, b).expect("240"), 240);
    refused(input::integer("slotMinutes", 4, b), "slotMinutes", "4");
    refused(input::integer("slotMinutes", 241, b), "slotMinutes", "241");
}

// ---------------------------------------------------------------- must_exist

async fn migrated_pool() -> (PgPool, String) {
    let url = std::env::var("TEST_DATABASE_URL").expect(
        "TEST_DATABASE_URL is not set; must_exist needs a real PostgreSQL and does not skip",
    );
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "the database name must end in `_test`"
    );
    let schema = xc_testdb::unique_schema_name("p1_input");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(options.clone())
        .await
        .expect("connect");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create schema");
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to schema");
    Migrator::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations")
            .as_path(),
    )
    .await
    .expect("read migrations")
    .run(&pool)
    .await
    .expect("apply migrations");
    (pool, schema)
}

async fn drop_schema(pool: PgPool, schema: String) {
    sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&pool)
        .await
        .expect("drop schema");
    pool.close().await;
}

#[actix_web::test]
async fn must_exist_accepts_a_currency_that_is_a_row_and_refuses_one_that_is_not() {
    let (pool, schema) = migrated_pool().await;
    let mut tx = pool.begin().await.expect("begin");
    input::must_exist(&mut tx, "currency", RowRef::Currency, "USD")
        .await
        .expect("USD is a currencies row");
    for code in ["XXX", "usd", "", "USDX"] {
        refused(
            input::must_exist(&mut tx, "currency", RowRef::Currency, code).await,
            "currency",
            &format!("{code:?}"),
        );
    }
    tx.rollback().await.expect("rollback");
    drop_schema(pool, schema).await;
}

#[actix_web::test]
async fn must_exist_reads_inside_the_callers_transaction() {
    // A row the caller inserted in its own, uncommitted transaction counts: the check runs on `tx`, not a new
    // connection.
    let (pool, schema) = migrated_pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_name = 'currencies' AND table_schema = current_schema() AND is_nullable = 'NO' \
         AND column_default IS NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .expect("read currencies columns");
    // Copy an existing row under a new code, so every NOT NULL column without a default is filled.
    let list = cols.join(", ");
    let select = cols
        .iter()
        .map(|c| {
            if c == "code" {
                "'ZZZ'".to_owned()
            } else {
                c.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO currencies ({list}) SELECT {select} FROM currencies WHERE code = 'USD'"
    )))
    .execute(&mut *tx)
    .await
    .expect("insert ZZZ in the transaction");
    input::must_exist(&mut tx, "currency", RowRef::Currency, "ZZZ")
        .await
        .expect("ZZZ exists inside this transaction");
    tx.rollback().await.expect("rollback");
    let mut tx = pool.begin().await.expect("begin again");
    refused(
        input::must_exist(&mut tx, "currency", RowRef::Currency, "ZZZ").await,
        "currency",
        "ZZZ after rollback",
    );
    tx.rollback().await.expect("rollback");
    drop_schema(pool, schema).await;
}

// ---------------------------------------------------------------- text (#4970 T1)
//
// Written before the implementation by the ticket author's lane (claude/28619d18, for claude/7ca80c).
// Do not edit, delete or weaken these tests; add your own in another file.
//
// #4970 §2.1: `input::text(field, text, TextBound { min, max })` counts chars after trim and refuses with
// `validationFailed` naming the field. Rows (T1): at max ok · max+1 refused · min-1 refused · padded with spaces
// is the trimmed count · chars, not bytes.

#[test]
fn text_at_max_is_accepted() {
    let b = input::TextBound { min: 1, max: 5 };
    assert!(input::text("name", "abcde", b).is_ok(), "5 chars at max 5");
    assert!(input::text("name", "a", b).is_ok(), "1 char at min 1");
}

#[test]
fn text_one_past_max_is_refused_on_the_field() {
    let b = input::TextBound { min: 1, max: 5 };
    refused(input::text("name", "abcdef", b), "name", "6 chars at max 5");
}

#[test]
fn text_one_below_min_is_refused_on_the_field() {
    let b = input::TextBound { min: 1, max: 5 };
    refused(input::text("reason", "", b), "reason", "empty at min 1");
    refused(input::text("reason", "   ", b), "reason", "blank at min 1");
    assert!(
        input::text("note", "", input::TextBound { min: 0, max: 5 }).is_ok(),
        "empty at min 0"
    );
}

#[test]
fn text_counts_after_trim() {
    let b = input::TextBound { min: 1, max: 5 };
    assert!(
        input::text("name", "   abcde   ", b).is_ok(),
        "5 chars padded with spaces at max 5"
    );
    refused(
        input::text("name", "  abcdef  ", b),
        "name",
        "6 chars padded with spaces at max 5",
    );
}

#[test]
fn text_counts_chars_not_bytes() {
    let b = input::TextBound { min: 1, max: 5 };
    assert!(
        input::text("name", &"ก".repeat(5), b).is_ok(),
        "5 Thai chars (15 bytes) at max 5"
    );
    refused(
        input::text("name", &"ก".repeat(6), b),
        "name",
        "6 Thai chars at max 5",
    );
}
