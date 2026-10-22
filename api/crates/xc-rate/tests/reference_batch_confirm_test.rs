//! XC-PRICING PX-12 (pricing-tiers.md §4, §11; design review FAIL issuecomment-6328842369 must-fix 1): a repeated
//! answer **confirms** its batch for a provider with `confirms_on_repeat()` (the market feeds), and nothing changes
//! for one without it (BoT). The provider here is a test-local one, so this file needs no other PX ticket.
//! Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not edit, delete or
//! weaken; add tests in a file of your own.

mod rate_engine_common;

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rate_engine_common::TestDb;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_rate::application::reference::reference_batch_writer::ReferenceBatchWriter;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::domain::reference::reference_provider::{ParsedBatch, ParsedLine, ReferenceProvider};
use xc_rate::infrastructure::reference::pg_reference_batch_store::PgReferenceBatchStore;

/// Answers whatever `next` holds, stamped with `at`.
struct Scripted {
    next: Mutex<(String, DateTime<Utc>)>,
}

impl Scripted {
    fn set(&self, buy: &str, at: DateTime<Utc>) {
        *self.next.lock().unwrap() = (buy.to_string(), at);
    }
}

impl RateSource for Scripted {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        let (buy, at) = self.next.lock().unwrap().clone();
        Box::pin(async move {
            Ok(ReferenceRates {
                source: "T".into(),
                fetched_at: at.to_rfc3339(),
                kind: "TEST".into(),
                rows: vec![json!({"code": "USD", "buy": buy, "sell": "40"})],
            })
        })
    }
}

/// A one-line provider; `confirms` decides `confirms_on_repeat`.
struct OneLine {
    name: &'static str,
    confirms: bool,
}

impl ReferenceProvider for OneLine {
    fn provider(&self) -> &'static str {
        self.name
    }
    fn reference_type(&self) -> &'static str {
        "MARKET_TT"
    }
    fn row_key(&self, row: &Value) -> String {
        row["code"].as_str().unwrap_or("").to_string()
    }
    fn parse(&self, rates: &ReferenceRates) -> Result<ParsedBatch, RateEngineError> {
        let r = &rates.rows[0];
        Ok(ParsedBatch {
            source_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(),
            source_published_at: None,
            complete: true,
            lines: vec![ParsedLine {
                currency: "USD".into(),
                unit_size: Decimal::ONE,
                buy: Some(Decimal::from_str(r["buy"].as_str().unwrap()).unwrap()),
                sell: Some(Decimal::from_str(r["sell"].as_str().unwrap()).unwrap()),
                raw: r.clone(),
                ..Default::default()
            }],
        })
    }
    fn confirms_on_repeat(&self) -> bool {
        self.confirms
    }
}

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 1, minute, 0).unwrap()
}

async fn write(db: &TestDb, src: &Arc<Scripted>, provider: OneLine) -> Uuid {
    let writer = ReferenceBatchWriter::new(src.clone(), Arc::new(PgReferenceBatchStore::new()))
        .with_provider(Arc::new(provider));
    let mut tx = db.pool.begin().await.unwrap();
    let id = writer.fetch_and_write(&mut tx).await.expect("written");
    tx.commit().await.unwrap();
    id
}

async fn batches(db: &TestDb, provider: &str) -> Vec<(Uuid, DateTime<Utc>)> {
    sqlx::query_as("SELECT id, fetched_at FROM rate_reference_batches WHERE provider = $1 ORDER BY fetched_at DESC, id DESC")
        .bind(provider)
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

fn src() -> Arc<Scripted> {
    Arc::new(Scripted { next: Mutex::new(("39".into(), at(0))) })
}

#[tokio::test]
async fn the_same_answer_again_confirms_its_batch() {
    let db = TestDb::start().await;
    let s = src();
    s.set("39", at(0));
    let first = write(&db, &s, OneLine { name: "PX12A", confirms: true }).await;
    s.set("39", at(35));
    let again = write(&db, &s, OneLine { name: "PX12A", confirms: true }).await;
    assert_eq!(again, first, "no new batch");
    assert_eq!(batches(&db, "PX12A").await, vec![(first, at(35))], "fetched_at = the last confirmation");
}

#[tokio::test]
async fn a_back_and_forth_answer_makes_the_old_batch_newest_again() {
    let db = TestDb::start().await;
    let s = src();
    s.set("39", at(0));
    let a = write(&db, &s, OneLine { name: "PX12B", confirms: true }).await;
    s.set("38", at(5));
    let b = write(&db, &s, OneLine { name: "PX12B", confirms: true }).await;
    s.set("39", at(10));
    let a_again = write(&db, &s, OneLine { name: "PX12B", confirms: true }).await;
    assert_eq!(a_again, a);
    assert_eq!(batches(&db, "PX12B").await, vec![(a, at(10)), (b, at(5))], "A is the newest line again; no third batch");
}

#[tokio::test]
async fn a_provider_that_does_not_confirm_is_unchanged() {
    let db = TestDb::start().await;
    let s = src();
    s.set("39", at(0));
    let first = write(&db, &s, OneLine { name: "PX12C", confirms: false }).await;
    s.set("39", at(35));
    assert_eq!(write(&db, &s, OneLine { name: "PX12C", confirms: false }).await, first);
    assert_eq!(batches(&db, "PX12C").await, vec![(first, at(0))], "BoT's behaviour: fetched_at untouched");
}
