//! Tests for rework hardening (PR #730 review):
//! 1. Cross-branch publish and copy of another branch's draft are refused (notFound),
//!    and both branches' publications and draft status remain unchanged.
//! 2. source_snapshot on published_rate_lines after AUTO publish contains:
//!    reference_line_id, policy_version_id, policy adjust %, and partner name/cost.
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f): RE-7 drops `rates` and moves
//! the publish's partner read to the newest `'PARTNER'` reference line of the currency
//! (`docs/design/blueprint/rates.md` §4.1 "Partner cost", §8 migration 6). So
//! `source_snapshot_carries_reference_policy_and_partner` seeds SuperRich's quote as a `'PARTNER'`
//! batch (provider `SuperRich`, USD per 1, sell 34.8) instead of a `rates` row, fetched an hour
//! before the BoT batch so the AUTO base is the BoT line either way, and reads the partner in
//! §4.1's keys: `provider` (was `name`) and `sell` (the partner's figure, was `cost`). The
//! reference, policy and adjust checks are unchanged.

mod rate_engine_common;

use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use futures::future::BoxFuture;
use pos_error::AppResult;
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_rate::application::reference::reference_batch_writer::ReferenceBatchWriter;
use xc_rate::domain::draft::rate_draft::DraftStatus;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::published_rates::PublishedRates;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::infrastructure::reference::pg_reference_batch_store::PgReferenceBatchStore;

#[tokio::test]
async fn cross_branch_publish_refused_and_main_draft_stays_approved() {
    let db = TestDb::start().await;
    let e = db.engine();
    db.seed_branch("B1").await;

    // Seed reference batch and policy for MAIN
    let now = Utc::now();
    db.seed_batch(now, &[("USD", "1", Some("35.0000"), Some("35.5000"))])
        .await;
    db.seed_policy("MAIN", "0.5000", "0.5000", json!({}), Some(24), None, None)
        .await;

    // Create an in-bound AUTO draft on MAIN and submit it
    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &owner(), vec![auto_line("USD")])
        .await
        .expect("draft created");
    tx.commit().await.expect("commit draft create");

    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner(), &draft.id, &key())
        .await
        .expect("submit succeeds");
    tx.commit().await.expect("commit submit");
    assert_eq!(outcome.status, DraftStatus::Approved);

    // Another branch (B1) tries to publish MAIN's approved draft
    let branch_b1 = BranchId::new("B1");
    let mut tx = db.begin().await;
    let publish_res = e
        .publisher
        .publish(&mut tx, &branch_b1, &owner(), &draft.id, None)
        .await;
    match publish_res {
        Err(RateEngineError::NotFound(_)) => (),
        other => panic!("expected NotFound on cross-branch publish, got {other:?}"),
    }
    tx.rollback().await.expect("rollback");

    // Verify MAIN's draft is STILL Approved
    let main_draft_after = e
        .drafts
        .get(&main_branch(), &draft.id)
        .await
        .expect("get draft")
        .expect("draft exists");
    assert_eq!(main_draft_after.status, DraftStatus::Approved);

    // Verify MAIN current publication is still None
    let main_pub = e
        .published
        .current(&main_branch())
        .await
        .expect("get current");
    assert!(main_pub.is_none());

    // Verify B1 has no publications
    let b1_pubs = db.publications("B1").await;
    assert!(b1_pubs.is_empty());

    db.finish().await;
}

#[tokio::test]
async fn cross_branch_copy_refused() {
    let db = TestDb::start().await;
    let e = db.engine();
    db.seed_branch("B1").await;

    let now = Utc::now();
    db.seed_batch(now, &[("USD", "1", Some("35.0000"), Some("35.5000"))])
        .await;
    db.seed_policy("MAIN", "0.5000", "0.5000", json!({}), Some(24), None, None)
        .await;

    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &owner(), vec![auto_line("USD")])
        .await
        .expect("draft created");
    tx.commit().await.expect("commit draft create");

    // B1 attempts to copy MAIN's draft
    let branch_b1 = BranchId::new("B1");
    let mut tx = db.begin().await;
    let copy_res = e
        .drafts
        .copy(&mut tx, &branch_b1, &owner(), &draft.id)
        .await;
    match copy_res {
        Err(RateEngineError::NotFound(_)) => (),
        other => panic!("expected NotFound on cross-branch copy, got {other:?}"),
    }
    tx.rollback().await.expect("rollback");

    // Verify no drafts were created for B1
    let b1_draft_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_drafts WHERE branch_id = 'B1'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(b1_draft_count, 0);

    db.finish().await;
}

#[tokio::test]
async fn source_snapshot_carries_reference_policy_and_partner() {
    let db = TestDb::start().await;
    let e = db.engine();

    let now = Utc::now();
    let batch_id = db
        .seed_batch(now, &[("USD", "1", Some("35.0000"), Some("35.5000"))])
        .await;

    let ref_line_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM rate_reference_lines WHERE batch_id = $1::uuid AND currency_code = 'USD'",
    )
    .bind(&batch_id)
    .fetch_one(&db.pool)
    .await
    .expect("ref line id");

    let pol_id_str = db
        .seed_policy("MAIN", "0.5000", "0.7500", json!({}), Some(24), None, None)
        .await;

    // Seed SuperRich's quote for USD as reference data (§4.1 "Partner cost"), older than BoT's.
    let partner_batch: String = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches \
           (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), 'SuperRich', ($1::timestamptz AT TIME ZONE 'UTC')::date, $1, $2, 'COMPLETE') \
         RETURNING id::text",
    )
    .bind(now - chrono::Duration::hours(1))
    .bind(unique("partner-checksum"))
    .fetch_one(&db.pool)
    .await
    .expect("seed a PARTNER batch");
    sqlx::query(
        "INSERT INTO rate_reference_lines \
           (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1::uuid, 'USD', 'PARTNER', 1, NULL, 34.8000, '{}'::jsonb)",
    )
    .bind(&partner_batch)
    .execute(&db.pool)
    .await
    .expect("seed the PARTNER line of USD");

    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &owner(), vec![auto_line("USD")])
        .await
        .expect("draft created");
    tx.commit().await.expect("commit draft create");

    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner(), &draft.id, &key())
        .await
        .expect("submit succeeds");
    tx.commit().await.expect("commit submit");
    assert_eq!(outcome.status, DraftStatus::Approved);

    let mut tx = db.begin().await;
    let pub_ref = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner(), &draft.id, None)
        .await
        .expect("publish succeeds");
    tx.commit().await.expect("commit publish");

    // Read back source_snapshot from published_rate_lines
    let snapshot: Value = sqlx::query_scalar(
        "SELECT source_snapshot FROM published_rate_lines \
         WHERE publication_id = $1::uuid AND currency_code = 'USD'",
    )
    .bind(pub_ref.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("source snapshot");

    // 1. reference_line_id
    assert_eq!(
        snapshot.get("reference_line_id").and_then(|v| v.as_str()),
        Some(ref_line_id.to_string().as_str()),
        "reference_line_id must match"
    );

    // 2. policy_version_id
    assert_eq!(
        snapshot.get("policy_version_id").and_then(|v| v.as_str()),
        Some(pol_id_str.as_str()),
        "policy_version_id must match"
    );

    // 3. policy adjust %
    let buy_pct = snapshot.get("buy_adjust_pct").unwrap();
    let sell_pct = snapshot.get("sell_adjust_pct").unwrap();
    assert_eq!(buy_pct.to_string().trim_matches('"'), "0.5000");
    assert_eq!(sell_pct.to_string().trim_matches('"'), "0.7500");

    // 4. partner: the PARTNER line's provider and figure (§4.1 keys)
    let partner = snapshot.get("partner").expect("partner present");
    assert_eq!(
        partner.get("provider").and_then(|v| v.as_str()),
        Some("SuperRich"),
        "partner.provider: {snapshot}"
    );
    let sell = partner
        .get("sell")
        .unwrap_or_else(|| panic!("partner.sell: {snapshot}"));
    let sell = rust_decimal::Decimal::from_str(sell.to_string().trim_matches('"'))
        .unwrap_or_else(|e| panic!("partner.sell {sell} is a decimal: {e}"));
    assert_eq!(sell, rust_decimal::Decimal::from_str("34.8").unwrap());

    db.finish().await;
}

struct FakeRateSource {
    rates: ReferenceRates,
}

impl FakeRateSource {
    fn new(rows: Vec<Value>) -> Self {
        Self {
            rates: ReferenceRates {
                source: "https://example.com/fake-bot".to_string(),
                fetched_at: "2026-10-09T10:00:00Z".to_string(),
                kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
                rows,
            },
        }
    }
}

impl RateSource for FakeRateSource {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        let rates = self.rates.clone();
        Box::pin(async move { Ok(rates) })
    }
}

fn fake_row(code: &str, unit: u64, date: &str, bt: &str, sell: &str) -> Value {
    json!({
        "code": code,
        "date": date,
        "unit": unit,
        "buyingSight": "30.0000",
        "buyingTransfer": bt,
        "selling": sell,
        "original": {
            "buyingSight": "30.0000",
            "buyingTransfer": bt,
            "selling": sell,
        }
    })
}

#[tokio::test]
async fn reference_batch_writer_writes_real_batch_and_deduplicates() {
    let db = TestDb::start().await;
    let store = Arc::new(PgReferenceBatchStore::new());

    let rows = vec![
        fake_row("USD", 1, "2026-10-09", "35.1000", "35.5000"),
        fake_row("EUR", 1, "2026-10-09", "38.2000", "38.7000"),
        fake_row("GBP", 1, "2026-10-09", "45.0000", "45.6000"),
        fake_row("JPY", 100, "2026-10-09", "23.4000", "23.8000"),
        fake_row("CNY", 1, "2026-10-09", "4.9000", "5.1000"),
        fake_row("AUD", 1, "2026-10-09", "23.0000", "23.5000"),
    ];

    let fake_source = Arc::new(FakeRateSource::new(rows.clone()));
    let writer = ReferenceBatchWriter::new(fake_source.clone(), store.clone());

    // First fetch: writes real batch and lines
    let mut tx = db.begin().await;
    let batch_id = writer
        .fetch_and_write(&mut tx)
        .await
        .expect("fetch and write succeeds");
    tx.commit().await.expect("commit tx");

    // Verify batch exists in rate_reference_batches
    let (status, provider, raw_checksum): (String, String, String) = sqlx::query_as(
        "SELECT status, provider, raw_checksum FROM rate_reference_batches WHERE id = $1",
    )
    .bind(batch_id)
    .fetch_one(&db.pool)
    .await
    .expect("batch exists");
    assert_eq!(status, "COMPLETE");
    assert_eq!(provider, "BOT");
    assert!(!raw_checksum.is_empty());

    // Verify lines exist in rate_reference_lines
    let line_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_reference_lines WHERE batch_id = $1")
            .bind(batch_id)
            .fetch_one(&db.pool)
            .await
            .expect("count lines");
    assert_eq!(line_count, 6);

    // Verify JPY unit_size is 100 (exactly as received, no unit conversion)
    let (jpy_unit, jpy_buy, jpy_sell, jpy_raw): (Decimal, Option<Decimal>, Option<Decimal>, Value) =
        sqlx::query_as(
            "SELECT unit_size, buy, sell, raw FROM rate_reference_lines WHERE batch_id = $1 AND currency_code = 'JPY'",
        )
        .bind(batch_id)
        .fetch_one(&db.pool)
        .await
        .expect("jpy line");
    assert_eq!(jpy_unit, Decimal::from(100));
    assert_eq!(jpy_buy, Some(Decimal::from_str("23.4000").unwrap()));
    assert_eq!(jpy_sell, Some(Decimal::from_str("23.8000").unwrap()));
    assert_eq!(jpy_raw.get("code").and_then(|v| v.as_str()), Some("JPY"));

    // Verify repeat fetch with identical data: returns SAME batch id and writes NOTHING
    let initial_batch_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_reference_batches")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let initial_line_count: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_reference_lines")
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let mut tx = db.begin().await;
    let repeat_id = writer
        .fetch_and_write(&mut tx)
        .await
        .expect("repeat fetch succeeds");
    tx.commit().await.expect("commit repeat tx");

    assert_eq!(
        repeat_id, batch_id,
        "repeat fetch must return the existing batch ID"
    );

    let final_batch_count: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_reference_batches")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let final_line_count: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_reference_lines")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        final_batch_count, initial_batch_count,
        "no new batch inserted"
    );
    assert_eq!(
        final_line_count, initial_line_count,
        "no new lines inserted"
    );

    // Partial fetch: missing one currency (e.g. AUD omitted)
    let partial_rows = vec![
        fake_row("USD", 1, "2026-10-10", "35.1000", "35.5000"),
        fake_row("EUR", 1, "2026-10-10", "38.2000", "38.7000"),
        fake_row("GBP", 1, "2026-10-10", "45.0000", "45.6000"),
        fake_row("JPY", 100, "2026-10-10", "23.4000", "23.8000"),
        fake_row("CNY", 1, "2026-10-10", "4.9000", "5.1000"),
    ];
    let partial_source = Arc::new(FakeRateSource::new(partial_rows));
    let partial_writer = ReferenceBatchWriter::new(partial_source, store.clone());

    let mut tx = db.begin().await;
    let partial_batch_id = partial_writer
        .fetch_and_write(&mut tx)
        .await
        .expect("partial fetch succeeds");
    tx.commit().await.expect("commit partial tx");

    assert_ne!(partial_batch_id, batch_id);
    let partial_status: String =
        sqlx::query_scalar("SELECT status FROM rate_reference_batches WHERE id = $1")
            .bind(partial_batch_id)
            .fetch_one(&db.pool)
            .await
            .expect("partial batch");
    assert_eq!(
        partial_status, "PARTIAL",
        "missing currency must give PARTIAL status"
    );

    let partial_line_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_reference_lines WHERE batch_id = $1")
            .bind(partial_batch_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(partial_line_count, 5);

    db.finish().await;
}

#[tokio::test]
async fn draft_simulation_reports_out_of_bound_and_leaves_tables_untouched() {
    let db = TestDb::start().await;
    let e = db.engine();

    let now = Utc::now();
    let _batch_id = db
        .seed_batch(now, &[("USD", "1", Some("35.0000"), Some("35.5000"))])
        .await;

    // Policy with ±1.0% guard on USD buy and sell
    let guards = json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        }
    });
    db.seed_policy("MAIN", "0.5000", "0.5000", guards, Some(24), None, None)
        .await;

    // 1. IN-BOUND draft: USD buy 35.1000, sell 35.6000 (well within ±1% distance of 35.0000/35.5000)
    let mut tx = db.begin().await;
    let in_bound_draft = e
        .drafts
        .create(
            &mut tx,
            &main_branch(),
            &owner(),
            vec![manual_line("USD", "35.1000", "35.6000")],
        )
        .await
        .expect("create in-bound draft");
    tx.commit().await.expect("commit create");

    // Simulate in-bound draft
    let mut tx = db.begin().await;
    let eval_res = e
        .drafts
        .evaluate(&mut tx, &main_branch(), &in_bound_draft.id)
        .await
        .expect("evaluate succeeds");
    tx.commit().await.expect("commit evaluate tx");

    assert!(
        eval_res.evaluation.is_in_bound(),
        "draft must be reported in-bound"
    );
    assert!(
        eval_res.evaluation.out_of_bound().is_empty(),
        "out_of_bound must be empty"
    );

    // Check draft status and approvals are untouched
    let d_row: (String, Option<Value>) =
        sqlx::query_as("SELECT status, evaluation FROM rate_drafts WHERE id = $1")
            .bind(in_bound_draft.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(d_row.0, "DRAFT", "draft status must remain DRAFT");
    assert!(d_row.1.is_none(), "evaluation column must remain NULL");

    let approval_count: i64 = sqlx::query_scalar("SELECT count(*) FROM approval_requests")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        approval_count, 0,
        "simulate must open zero approval requests"
    );

    // 2. OUT-OF-BOUND draft: USD sell 50.0000 (distance ~41%, far exceeding max_pct 1.0%)
    let mut tx = db.begin().await;
    let out_bound_draft = e
        .drafts
        .create(
            &mut tx,
            &main_branch(),
            &owner(),
            vec![manual_line("USD", "35.1000", "50.0000")],
        )
        .await
        .expect("create out-of-bound draft");
    tx.commit().await.expect("commit create");

    // Simulate out-of-bound draft
    let mut tx = db.begin().await;
    let eval_out = e
        .drafts
        .evaluate(&mut tx, &main_branch(), &out_bound_draft.id)
        .await
        .expect("evaluate succeeds");
    tx.commit().await.expect("commit evaluate tx");

    assert!(
        !eval_out.evaluation.is_in_bound(),
        "draft must be reported out-of-bound"
    );
    let out_list = eval_out.evaluation.out_of_bound();
    assert_eq!(out_list.len(), 1);
    assert_eq!(out_list[0].0.as_str(), "USD");
    assert_eq!(out_list[0].1, Side::Sell);

    // Formatted strings match "USD:Sell"
    let out_strings: Vec<String> = out_list
        .iter()
        .map(|(c, s)| format!("{}:{:?}", c.as_str(), s))
        .collect();
    assert_eq!(out_strings, vec!["USD:Sell"]);

    // Check draft status and approvals remain untouched
    let d_out_row: (String, Option<Value>) =
        sqlx::query_as("SELECT status, evaluation FROM rate_drafts WHERE id = $1")
            .bind(out_bound_draft.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        d_out_row.0, "DRAFT",
        "out-of-bound draft status must remain DRAFT"
    );
    assert!(
        d_out_row.1.is_none(),
        "out-of-bound draft evaluation column must remain NULL"
    );

    let approval_count_2: i64 = sqlx::query_scalar("SELECT count(*) FROM approval_requests")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        approval_count_2, 0,
        "simulate must open zero approval requests"
    );

    db.finish().await;
}
