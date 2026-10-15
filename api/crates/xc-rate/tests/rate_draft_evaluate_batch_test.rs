//! Ticket #1988 — `RateDraftService::evaluate` batches reference-line and currency-unit lookups
//! while keeping identical evaluation behaviour across all 5 behaviour-table rows.

mod rate_engine_common;

use chrono::{Duration, Utc};
use rate_engine_common::*;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access;
use xc_rate::application::draft::rate_draft_service::DraftEvaluationOutcome;
use xc_rate::domain::draft::rate_draft::{DraftLine, RateDraft};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;

async fn settle<T>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, RateEngineError>,
) -> Result<T, &'static str> {
    match result {
        Ok(value) => {
            tx.commit().await.expect("commit");
            Ok(value)
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e.code())
        }
    }
}

async fn create_draft(
    db: &TestDb,
    e: &Engine,
    who: &access::Actor,
    lines: Vec<DraftLine>,
) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e.drafts.create(&mut tx, &main_branch(), who, lines).await;
    settle(tx, got).await.expect("draft created")
}

/// Assign deterministic ascending `id` UUIDs to `rate_draft_lines` in the order of `codes`,
/// since `RateDraftService::evaluate` loads `rate_draft_lines` with `ORDER BY id`.
async fn order_draft_lines(db: &TestDb, draft: &RateDraft, codes: &[&str]) {
    for (idx, code) in codes.iter().enumerate() {
        let deterministic_id = format!("00000000-0000-0000-0000-{:012}", idx + 1);
        sqlx::query(
            "UPDATE rate_draft_lines SET id = $1::uuid WHERE draft_id = $2 AND currency_code = $3",
        )
        .bind(&deterministic_id)
        .bind(draft.id.as_uuid())
        .bind(*code)
        .execute(&db.pool)
        .await
        .expect("order rate_draft_lines deterministically");
    }
}

async fn evaluate_draft(db: &TestDb, e: &Engine, draft: &RateDraft) -> DraftEvaluationOutcome {
    let mut tx = db.begin().await;
    let got = e.drafts.evaluate(&mut tx, &main_branch(), &draft.id).await;
    settle(tx, got).await.expect("draft evaluated")
}

fn guard(min: &str, max: &str) -> Value {
    json!({ "min_pct": min, "max_pct": max })
}

/// Row 1: Draft with lines on 3 references (USD, EUR, JPY) -> `references` has the same 3 elements
/// in `ref_ids` order (first appearance in the lines).
/// Red under: rebuild in SQL return order, or `ORDER BY` code.
#[tokio::test]
async fn draft_with_three_references_rebuilds_references_in_ref_ids_order() {
    let db = TestDb::start().await;
    let e = db.engine();

    let fetched_at = Utc::now() - Duration::minutes(15);
    // Insert reference lines in alphabetical / physical order: EUR, JPY, USD.
    db.seed_batch(
        fetched_at,
        &[
            ("EUR", "1", Some("36.00"), Some("37.00")),
            ("JPY", "100", Some("24.00"), Some("25.00")),
            ("USD", "1", Some("32.00"), Some("33.00")),
        ],
    )
    .await;

    db.seed_policy(
        MAIN,
        "0.0000",
        "0.0000",
        json!({
            "USD": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
            "EUR": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
            "JPY": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
        }),
        Some(24),
        None,
        None,
    )
    .await;

    // Draft lines ordered: USD, EUR, JPY (distinct from both physical insertion order and alphabetical order).
    let draft = create_draft(
        &db,
        &e,
        &owner_user(),
        vec![auto_line("USD"), auto_line("EUR"), auto_line("JPY")],
    )
    .await;
    order_draft_lines(&db, &draft, &["USD", "EUR", "JPY"]).await;

    let outcome = evaluate_draft(&db, &e, &draft).await;
    assert_eq!(outcome.ref_ids.len(), 3);
    assert_eq!(outcome.references.len(), 3);

    let ref_codes: Vec<&str> = outcome
        .references
        .iter()
        .map(|r| r.currency.as_str())
        .collect();
    assert_eq!(
        ref_codes,
        vec!["USD", "EUR", "JPY"],
        "references must be in ref_ids order (first appearance in lines)"
    );

    db.finish().await;
}

/// Row 2: One `ref_id` whose row no longer exists -> skipped, the others kept, no error.
/// Red under: erroring on a missing row.
#[tokio::test]
async fn missing_reference_line_row_is_skipped_and_others_kept_without_error() {
    let db = TestDb::start().await;
    let e = db.engine();

    let fetched_at = Utc::now() - Duration::minutes(15);
    db.seed_batch(
        fetched_at,
        &[
            ("USD", "1", Some("32.00"), Some("33.00")),
            ("EUR", "1", Some("36.00"), Some("37.00")),
            ("JPY", "100", Some("24.00"), Some("25.00")),
        ],
    )
    .await;

    db.seed_policy(
        MAIN,
        "0.0000",
        "0.0000",
        json!({
            "USD": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
            "EUR": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
            "JPY": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
        }),
        Some(24),
        None,
        None,
    )
    .await;

    let draft = create_draft(
        &db,
        &e,
        &owner_user(),
        vec![auto_line("USD"), auto_line("EUR"), auto_line("JPY")],
    )
    .await;
    order_draft_lines(&db, &draft, &["USD", "EUR", "JPY"]).await;

    // Remove the EUR reference row while keeping its UUID in rate_draft_lines.reference_line_id.
    sqlx::query(
        "ALTER TABLE rate_draft_lines DROP CONSTRAINT rate_draft_lines_reference_line_id_fkey",
    )
    .execute(&db.pool)
    .await
    .expect("drop reference_line_id fk for missing-row test");
    sqlx::query("DELETE FROM rate_reference_lines WHERE currency_code = 'EUR'")
        .execute(&db.pool)
        .await
        .expect("delete EUR reference row");

    let outcome = evaluate_draft(&db, &e, &draft).await;
    assert_eq!(
        outcome.ref_ids.len(),
        3,
        "all 3 ref_ids are still present on the draft lines"
    );
    let ref_codes: Vec<&str> = outcome
        .references
        .iter()
        .map(|r| r.currency.as_str())
        .collect();
    assert_eq!(
        ref_codes,
        vec!["USD", "JPY"],
        "missing EUR reference row must be skipped and USD/JPY kept in ref_ids order"
    );
    assert_eq!(outcome.proposals[0].buy, Some(rate("32.00")));
    assert_eq!(outcome.proposals[1].buy, None);
    assert_eq!(outcome.proposals[2].buy, Some(rate("24.00")));

    db.finish().await;
}

/// Row 3: A line with JPY `unit_size` 100 next to USD `unit_size` 1 -> each line's `CurrencyUnits`
/// uses its own currency's unit and minor unit.
/// Red under: looking up the first row for every line.
#[tokio::test]
async fn jpy_unit_size_100_next_to_usd_unit_size_1_uses_each_currencys_own_units() {
    let db = TestDb::start().await;
    let e = db.engine();

    let fetched_at = Utc::now() - Duration::minutes(15);
    // Both references are given per 1 unit in the reference batch (unit_size = 1):
    // USD: 32.00 / 33.00 per 1 USD -> with currencies.unit_size = 1, proposed is 32.00 / 33.00.
    // JPY: 0.24 / 0.25 per 1 JPY -> with currencies.unit_size = 100, proposed is 24.00 / 25.00.
    db.seed_batch(
        fetched_at,
        &[
            ("USD", "1", Some("32.00"), Some("33.00")),
            ("JPY", "1", Some("0.24"), Some("0.25")),
        ],
    )
    .await;

    db.seed_policy(
        MAIN,
        "0.0000",
        "0.0000",
        json!({
            "USD": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
            "JPY": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
        }),
        Some(24),
        None,
        None,
    )
    .await;

    let draft = create_draft(
        &db,
        &e,
        &owner_user(),
        vec![auto_line("JPY"), auto_line("USD")],
    )
    .await;
    order_draft_lines(&db, &draft, &["JPY", "USD"]).await;

    let outcome = evaluate_draft(&db, &e, &draft).await;
    assert_eq!(outcome.proposals.len(), 2);
    // Line 0 (JPY, unit_size 100): 0.24 * 100 / 1 = 24.00, 0.25 * 100 / 1 = 25.00
    assert_eq!(outcome.proposals[0].buy, Some(rate("24.00")));
    assert_eq!(outcome.proposals[0].sell, Some(rate("25.00")));
    // Line 1 (USD, unit_size 1): 32.00 * 1 / 1 = 32.00, 33.00 * 1 / 1 = 33.00
    assert_eq!(outcome.proposals[1].buy, Some(rate("32.00")));
    assert_eq!(outcome.proposals[1].sell, Some(rate("33.00")));

    db.finish().await;
}

/// Row 4: A line whose currency has no `currencies` row -> falls back to `unit_size 1`, `minor_unit 2`.
/// Red under: dropping the fallback.
#[tokio::test]
async fn line_without_currencies_row_falls_back_to_unit_size_1_and_minor_unit_2() {
    let db = TestDb::start().await;
    let e = db.engine();

    let fetched_at = Utc::now() - Duration::minutes(15);
    // Seed CHF reference with unit_size = 100, buy = 3800.00, sell = 3900.00.
    // When CHF has no row in `currencies` and falls back to unit_size = 1, minor_unit = 2:
    // proposed buy = 3800.00 * 1 / 100 = 38.00, sell = 3900.00 * 1 / 100 = 39.00.
    db.seed_batch(
        fetched_at,
        &[("CHF", "100", Some("3800.00"), Some("3900.00"))],
    )
    .await;

    db.seed_policy(
        MAIN,
        "0.0000",
        "0.0000",
        json!({
            "CHF": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
        }),
        Some(24),
        None,
        None,
    )
    .await;

    let draft = create_draft(&db, &e, &owner_user(), vec![auto_line("CHF")]).await;

    // Remove CHF from `currencies` in this test schema so `evaluate` finds no `currencies` row for CHF.
    sqlx::query("ALTER TABLE rate_draft_lines DROP CONSTRAINT rate_draft_lines_currency_code_fkey")
        .execute(&db.pool)
        .await
        .expect("drop rate_draft_lines currency_code fk");
    sqlx::query(
        "ALTER TABLE rate_reference_lines DROP CONSTRAINT rate_reference_lines_currency_code_fkey",
    )
    .execute(&db.pool)
    .await
    .expect("drop rate_reference_lines currency_code fk");
    sqlx::query("DELETE FROM denominations WHERE currency_code = 'CHF'")
        .execute(&db.pool)
        .await
        .expect("delete CHF denominations");
    sqlx::query("DELETE FROM currencies WHERE code = 'CHF'")
        .execute(&db.pool)
        .await
        .expect("delete CHF from currencies");

    let outcome = evaluate_draft(&db, &e, &draft).await;
    assert_eq!(outcome.proposals.len(), 1);
    assert_eq!(outcome.proposals[0].buy, Some(rate("38.00")));
    assert_eq!(outcome.proposals[0].sell, Some(rate("39.00")));

    db.finish().await;
}

/// Row 5: Draft with 0 lines -> evaluation result identical to today.
#[tokio::test]
async fn draft_with_zero_lines_evaluates_identically_to_today() {
    let db = TestDb::start().await;
    let e = db.engine();

    let draft = create_draft(&db, &e, &owner_user(), vec![]).await;
    let outcome = evaluate_draft(&db, &e, &draft).await;

    assert_eq!(outcome.version, 1);
    assert_eq!(outcome.status, "DRAFT");
    assert!(outcome.lines.is_empty());
    assert!(outcome.proposals.is_empty());
    assert!(outcome.references.is_empty());
    assert!(outcome.ref_ids.is_empty());
    assert!(outcome.tiers.is_empty());
    assert!(outcome.tier_lines.is_empty());
    assert!(outcome.policy.is_none());
    assert!(outcome.evaluation.is_in_bound());
    assert!(outcome.evaluation.out_of_bound().is_empty());

    db.finish().await;
}
