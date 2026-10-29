//! Database helpers of the pricing-formula up-front tests (on `rate_engine_common::TestDb`). Written before the
//! implementation. Do not edit, delete or weaken.
#![allow(dead_code)]

use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use super::rate_engine_common::{TestDb, unique};

pub const SEEDED_ORG: &str = "00000000-0000-0000-0000-000000000001";

pub fn org() -> Uuid {
    Uuid::parse_str(SEEDED_ORG).expect("the seeded organisation id")
}

/// Guards of ±50 % for every currency the tests use.
pub fn wide_guards() -> Value {
    guards("-50.0000", "50.0000")
}

pub fn guards(min: &str, max: &str) -> Value {
    let g = json!({ "min_pct": min, "max_pct": max });
    json!({
        "USD": { "buy": g, "sell": g },
        "EUR": { "buy": g, "sell": g },
        "GBP": { "buy": g, "sell": g },
        "JPY": { "buy": g, "sell": g },
    })
}

/// A BOT batch fetched an hour ago with the 10-28 USD line and an EUR line.
pub async fn bot_usd_eur(db: &TestDb) {
    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[
            ("USD", "1", Some("33.6482"), Some("33.9671")),
            ("EUR", "1", Some("38.0000"), Some("38.6000")),
        ],
    )
    .await;
}

/// One market line (`MARKET_TT` / `MARKET_SR`) for `currency`, per 1 unit, fetched now.
pub async fn market_line(db: &TestDb, currency: &str, reference_type: &str, buy: &str, sell: &str) {
    let batch: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), $1, CURRENT_DATE, now(), $2, 'COMPLETE') RETURNING id",
    )
    .bind(reference_type)
    .bind(unique("chk"))
    .fetch_one(&db.pool)
    .await
    .expect("insert a market batch");
    sqlx::query(
        "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1, $2, $3, 1, $4::numeric, $5::numeric, '{}'::jsonb)",
    )
    .bind(batch)
    .bind(currency)
    .bind(reference_type)
    .bind(buy)
    .bind(sell)
    .execute(&db.pool)
    .await
    .expect("insert a market line");
}

/// The branch's current policy version id (seeded through `TestDb::seed_policy`).
pub async fn policy(db: &TestDb, branch: &str, buy: &str, sell: &str, guards: Value) -> Uuid {
    let id = db.seed_policy(branch, buy, sell, guards, None, None, None).await;
    Uuid::parse_str(&id).expect("a uuid")
}

/// Gives policy version `id` its own formula.
pub async fn set_branch_formula(db: &TestDb, id: Uuid, buy: &str, sell: &str) {
    sqlx::query("UPDATE rate_policy_versions SET buy_formula = $2, sell_formula = $3 WHERE id = $1")
        .bind(id)
        .bind(buy)
        .bind(sell)
        .execute(&db.pool)
        .await
        .expect("set the branch formula");
}

/// An organisation formula version of the seeded organisation; `current` = `valid_to IS NULL`.
pub async fn org_formula(db: &TestDb, version: i32, buy: &str, sell: &str, current: bool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pricing_formula_versions \
         (id, organization_id, version, buy_formula, sell_formula, valid_from, valid_to, approved_by, reason) \
         VALUES ($1, $2, $3, $4, $5, now() - interval '1 hour', CASE WHEN $6 THEN NULL ELSE now() END, 'exec-1', 'test')",
    )
    .bind(id)
    .bind(org())
    .bind(version)
    .bind(buy)
    .bind(sell)
    .bind(current)
    .execute(&db.pool)
    .await
    .expect("insert an organisation formula");
    id
}

/// `pricing_modes`: `currency` in FOLLOW (MAIN only, pricing-tiers.md).
pub async fn follow(db: &TestDb, currency: &str) {
    sqlx::query(
        "INSERT INTO pricing_modes (currency_code, mode, revision, updated_by, updated_at) \
         VALUES ($1, 'FOLLOW', 1, 'exec-1', now())",
    )
    .bind(currency)
    .execute(&db.pool)
    .await
    .expect("set FOLLOW");
}
