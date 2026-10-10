//! Rate feed F1 — the `ReferenceProvider` seam (`docs/design/rate-feed-connector.md` §2, §3.1).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` for the writer tests (see `rate_engine_common/mod.rs`).
//!
//! What this pins:
//! - `xc_rate::domain::reference::reference_provider::{ReferenceProvider, ParsedBatch, ParsedLine}`:
//!   `ReferenceProvider: Send + Sync` with `provider() -> &'static str`,
//!   `reference_type() -> &'static str`, `row_key(&Value) -> String`,
//!   `parse(&ReferenceRates) -> Result<ParsedBatch, RateEngineError>`.
//!   `ParsedBatch { source_date, source_published_at, complete, lines }` and
//!   `ParsedLine { currency, instrument_key, value_tier, unit_size, base_buy, base_sell, buy, sell, raw }`
//!   both derive `Default` (this file builds them with `..Default::default()`, so fields added later
//!   do not touch it).
//! - `xc_rate::infrastructure::reference::bot_provider::BotProvider` (a unit struct): today's BoT
//!   reading of `reference_batch_writer.rs`, moved and unchanged.
//! - `ReferenceBatchWriter::new(source, store)` still means BoT; `.with_provider(p)` swaps the
//!   provider. The writer sorts the checksum by `p.row_key`, looks for a repeat checksum **of
//!   `p.provider()`**, and writes `p.provider()` / `p.reference_type()`.

mod rate_engine_common;

use std::str::FromStr;
use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_rate::application::reference::reference_batch_writer::ReferenceBatchWriter;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::domain::reference::reference_provider::{ParsedBatch, ParsedLine, ReferenceProvider};
use xc_rate::infrastructure::reference::bot_provider::BotProvider;
use xc_rate::infrastructure::reference::pg_reference_batch_store::PgReferenceBatchStore;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn rates(fetched_at: &str, rows: Vec<Value>) -> ReferenceRates {
    ReferenceRates {
        source: "https://example.com/test".to_string(),
        fetched_at: fetched_at.to_string(),
        kind: "TEST".to_string(),
        rows,
    }
}

fn bot_row(code: &str, unit: u64, date: &str, bt: &str, sell: &str) -> Value {
    json!({ "code": code, "date": date, "unit": unit, "buyingSight": "30.0000",
            "buyingTransfer": bt, "selling": sell })
}

fn six_bot_rows() -> Vec<Value> {
    vec![
        bot_row("USD", 1, "2026-10-09", "35.1000", "35.5000"),
        bot_row("EUR", 1, "2026-10-09", "38.2000", "38.7000"),
        bot_row("GBP", 1, "2026-10-09", "45.0000", "45.6000"),
        bot_row("JPY", 100, "2026-10-09", "23.4000", "23.8000"),
        bot_row("CNY", 1, "2026-10-09", "4.9000", "5.1000"),
        bot_row("AUD", 1, "2026-10-09", "23.0000", "23.5000"),
    ]
}

// ── BotProvider: today's BoT reading, moved ─────────────────────────────────────────────────

#[test]
fn the_bot_provider_names_itself_bot() {
    let p = BotProvider;
    assert_eq!(p.provider(), "BOT");
    assert_eq!(p.reference_type(), "BOT");
}

#[test]
fn the_bot_row_key_is_the_trimmed_upper_cased_code() {
    let p = BotProvider;
    assert_eq!(p.row_key(&json!({ "code": " usd " })), "USD");
    assert_eq!(p.row_key(&json!({ "currency_code": "eur" })), "EUR");
    assert_eq!(
        p.row_key(&json!({ "code": "jpy", "currency_code": "XXX" })),
        "JPY",
        "code wins"
    );
    assert_eq!(
        p.row_key(&json!({ "selling": "1" })),
        "",
        "a row with no code keys as empty"
    );
}

#[test]
fn the_bot_provider_parses_bot_rows_as_the_writer_did() {
    let mut rows = six_bot_rows();
    // a second USD row: today the writer keeps the first one per currency (`:129`)
    rows.push(bot_row("USD", 1, "2026-10-09", "99.0000", "99.9000"));
    // a row with no code: skipped
    rows.push(json!({ "date": "2026-10-09", "buyingTransfer": "1", "selling": "2" }));
    // the fallback keys `buy` / `sell` and a string unit
    rows[1] = json!({ "code": "eur", "date": "2026-10-09", "unit": "1", "buy": "38.2000", "sell": "38.7000" });

    let batch = BotProvider
        .parse(&rates("2026-10-09T10:00:00Z", rows.clone()))
        .expect("BoT rows parse");

    assert!(batch.complete, "all six BoT currencies are present");
    assert_eq!(
        batch.source_date,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 9).unwrap()
    );
    assert_eq!(
        batch.source_published_at, None,
        "BoT gives no announcement time"
    );
    let codes: Vec<&str> = batch.lines.iter().map(|l| l.currency.as_str()).collect();
    assert_eq!(
        codes,
        ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"],
        "first row per currency, in order"
    );

    let usd = &batch.lines[0];
    assert_eq!(
        (usd.buy, usd.sell),
        (Some(dec("35.1000")), Some(dec("35.5000"))),
        "the first USD row"
    );
    assert_eq!(usd.unit_size, Decimal::ONE);
    assert_eq!(usd.raw, rows[0], "raw is the row as received");
    assert_eq!(
        (usd.instrument_key.as_deref(), usd.value_tier.as_deref()),
        (None, None)
    );
    assert_eq!((usd.base_buy, usd.base_sell), (None, None));

    let eur = &batch.lines[1];
    assert_eq!(eur.currency, "EUR", "upper-cased");
    assert_eq!(
        (eur.buy, eur.sell),
        (Some(dec("38.2000")), Some(dec("38.7000"))),
        "buy / sell keys"
    );

    let jpy = &batch.lines[3];
    assert_eq!(jpy.unit_size, dec("100"), "the source unit as received");
    assert_eq!(
        (jpy.buy, jpy.sell),
        (Some(dec("23.4000")), Some(dec("23.8000"))),
        "no unit conversion"
    );
}

#[test]
fn a_bot_batch_missing_a_currency_is_partial_and_dates_from_the_fetch() {
    let rows: Vec<Value> = six_bot_rows()
        .into_iter()
        .filter(|r| r["code"] != "AUD")
        .map(|mut r| {
            r.as_object_mut().unwrap().remove("date");
            r
        })
        .collect();
    let batch = BotProvider
        .parse(&rates("2026-10-08T23:30:00Z", rows))
        .expect("parse");
    assert!(!batch.complete, "AUD is missing");
    assert_eq!(batch.lines.len(), 5);
    assert_eq!(
        batch.source_date,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
        "fetched_at's date"
    );
}

#[test]
fn a_bot_period_dates_the_batch() {
    let rows = vec![
        json!({ "code": "USD", "period": "2026-10-07", "buyingTransfer": "35", "selling": "36" }),
    ];
    let batch = BotProvider
        .parse(&rates("2026-10-09T10:00:00Z", rows))
        .expect("parse");
    assert_eq!(
        batch.source_date,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 7).unwrap()
    );
}

// ── the writer, through the seam ────────────────────────────────────────────────────────────

struct FixedSource(ReferenceRates);

impl RateSource for FixedSource {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        let r = self.0.clone();
        Box::pin(async move { Ok(r) })
    }
}

/// A provider that keys rows by `code/key` and turns only the **first** row into a USD line, so
/// no test here needs the identity columns of the next migration.
struct FakeProvider;

impl ReferenceProvider for FakeProvider {
    fn provider(&self) -> &'static str {
        "FAKE"
    }
    fn reference_type(&self) -> &'static str {
        "FAKE_T"
    }
    fn row_key(&self, row: &Value) -> String {
        format!(
            "{}/{}",
            row["code"].as_str().unwrap_or_default(),
            row["key"].as_str().unwrap_or_default()
        )
    }
    fn parse(&self, rates: &ReferenceRates) -> Result<ParsedBatch, RateEngineError> {
        let first = rates.rows.first().cloned().unwrap_or(Value::Null);
        Ok(ParsedBatch {
            source_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 10).unwrap(),
            complete: true,
            lines: vec![ParsedLine {
                currency: "USD".to_string(),
                unit_size: Decimal::ONE,
                buy: Some(dec("32.1")),
                sell: Some(dec("33.0")),
                raw: first,
                ..Default::default()
            }],
            ..Default::default()
        })
    }
}

fn writer(rows: Vec<Value>, fake: bool) -> ReferenceBatchWriter {
    let source = Arc::new(FixedSource(rates("2026-10-10T03:00:00Z", rows)));
    let w = ReferenceBatchWriter::new(source, Arc::new(PgReferenceBatchStore::new()));
    if fake {
        w.with_provider(Arc::new(FakeProvider))
    } else {
        w
    }
}

async fn write(db: &TestDb, w: &ReferenceBatchWriter) -> uuid::Uuid {
    let mut tx = db.begin().await;
    let id = w.fetch_and_write(&mut tx).await.expect("fetch_and_write");
    tx.commit().await.expect("commit");
    id
}

#[tokio::test]
async fn new_still_means_bot() {
    let db = TestDb::start().await;
    let id = write(&db, &writer(six_bot_rows(), false)).await;
    let (provider, status): (String, String) =
        sqlx::query_as("SELECT provider, status FROM rate_reference_batches WHERE id = $1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .expect("batch");
    assert_eq!((provider.as_str(), status.as_str()), ("BOT", "COMPLETE"));
    let types: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT reference_type FROM rate_reference_lines WHERE batch_id = $1",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("types");
    assert_eq!(types, ["BOT"]);
    db.finish().await;
}

#[tokio::test]
async fn with_provider_writes_the_providers_names_and_lines() {
    let db = TestDb::start().await;
    let row = json!({ "code": "USD", "key": "a" });
    let id = write(&db, &writer(vec![row.clone()], true)).await;

    let (provider, status, date): (String, String, chrono::NaiveDate) = sqlx::query_as(
        "SELECT provider, status, source_date FROM rate_reference_batches WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("batch");
    assert_eq!(provider, "FAKE");
    assert_eq!(status, "COMPLETE");
    assert_eq!(
        date,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 10).unwrap(),
        "the provider's source_date"
    );

    let lines: Vec<(String, String, Option<Decimal>, Option<Decimal>, Value)> = sqlx::query_as(
        "SELECT currency_code, reference_type, buy, sell, raw FROM rate_reference_lines WHERE batch_id = $1",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("lines");
    assert_eq!(lines.len(), 1, "exactly the provider's lines");
    assert_eq!(lines[0].0.trim(), "USD");
    assert_eq!(lines[0].1, "FAKE_T");
    assert_eq!(
        (lines[0].2, lines[0].3),
        (Some(dec("32.1")), Some(dec("33.0")))
    );
    assert_eq!(lines[0].4, row);
    db.finish().await;
}

#[tokio::test]
async fn a_repeat_checksum_is_looked_up_per_provider() {
    let db = TestDb::start().await;
    let rows = six_bot_rows();
    let bot = write(&db, &writer(rows.clone(), false)).await;
    let fake = write(&db, &writer(rows.clone(), true)).await;
    assert_ne!(
        bot, fake,
        "the same rows from another provider are another batch"
    );
    let fake_again = write(&db, &writer(rows, true)).await;
    assert_eq!(
        fake, fake_again,
        "a repeat from the same provider writes nothing and answers its batch"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_checksum_is_sorted_by_the_providers_row_key() {
    let db = TestDb::start().await;
    let b = json!({ "code": "USD", "key": "b" });
    let a = json!({ "code": "USD", "key": "a" });
    let first = write(&db, &writer(vec![b.clone(), a.clone()], true)).await;
    let second = write(&db, &writer(vec![a, b], true)).await;
    assert_eq!(
        first, second,
        "two rows of one currency in another order are the same content under the provider's row_key"
    );
    db.finish().await;
}
