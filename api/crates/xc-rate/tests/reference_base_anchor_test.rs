//! PT-15 (pricing-tiers.md §3.1, §11; owner P-4 "configurable"): `ReferenceBase::newest_in` reads the effective
//! anchor's reference type. The effective anchor of (branch, currency) is the currency's
//! `rate_policy_currency_settings.anchor_source` on the branch's current policy version if set, else that version's
//! `rate_policy_versions.anchor_source`, else `BOT` (no version). `START` stays the fallback of the `BOT` anchor only.
//!
//! Written before the implementation by the ticket author (claude/647e1967, PT-0). Do not edit, delete or weaken;
//! add tests in a file of your own. `reference_base_test.rs` (SP-2) must stay green as it is.
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database ending in `_test`.

mod rate_engine_common;

use chrono::{DateTime, Duration, Utc};
use rate_engine_common::{MAIN, ORG, TestDb};
use serde_json::json;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::draft::reference_base::ReferenceBase;

/// One batch of `provider` with one line of `reference_type` for `currency`, fetched at `fetched_at`.
async fn line(
    db: &TestDb,
    currency: &str,
    reference_type: &str,
    provider: &str,
    fetched_at: DateTime<Utc>,
) -> Uuid {
    let batch: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), $1, CURRENT_DATE, $2, $3, 'COMPLETE') RETURNING id",
    )
    .bind(provider)
    .bind(fetched_at)
    .bind(format!("chk-{}", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .expect("insert rate_reference_batches");
    sqlx::query_scalar(
        "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1, $2, $3, 1.0, 35.0, 36.0, '{}'::jsonb) RETURNING id",
    )
    .bind(batch)
    .bind(currency)
    .bind(reference_type)
    .fetch_one(&db.pool)
    .await
    .expect("insert rate_reference_lines")
}

async fn bot(db: &TestDb, currency: &str, hours_ago: i64) -> Uuid {
    line(
        db,
        currency,
        "BOT",
        "BOT",
        Utc::now() - Duration::hours(hours_ago),
    )
    .await
}

async fn tt(db: &TestDb, currency: &str, hours_ago: i64) -> Uuid {
    line(
        db,
        currency,
        "MARKET_TT",
        "TT",
        Utc::now() - Duration::hours(hours_ago),
    )
    .await
}

async fn sr(db: &TestDb, currency: &str, hours_ago: i64) -> Uuid {
    line(
        db,
        currency,
        "MARKET_SR",
        "SR",
        Utc::now() - Duration::hours(hours_ago),
    )
    .await
}

async fn start(db: &TestDb, currency: &str, hours_ago: i64) -> Uuid {
    line(
        db,
        currency,
        "START",
        &format!("START:{ORG}"),
        Utc::now() - Duration::hours(hours_ago),
    )
    .await
}

/// `MAIN` (seeded by the migrations, in the seeded organisation) with policy version 1 anchored on `branch_anchor`.
async fn branch_anchored(db: &TestDb, branch_anchor: &str) -> Uuid {
    let id = db
        .seed_policy(MAIN, "0", "0", json!({}), None, None, None)
        .await;
    let id = Uuid::parse_str(&id).expect("uuid");
    sqlx::query("UPDATE rate_policy_versions SET anchor_source = $2 WHERE id = $1")
        .bind(id)
        .bind(branch_anchor)
        .execute(&db.pool)
        .await
        .expect("set the version's anchor");
    id
}

async fn currency_anchored(db: &TestDb, version: Uuid, currency: &str, anchor: Option<&str>) {
    sqlx::query(
        "INSERT INTO rate_policy_currency_settings (policy_version_id, currency_code, min_spread_pct, anchor_source) \
         VALUES ($1, $2, NULL, $3)",
    )
    .bind(version)
    .bind(currency)
    .bind(anchor)
    .execute(&db.pool)
    .await
    .expect("insert rate_policy_currency_settings");
}

async fn base(db: &TestDb, currency: &str) -> Option<Uuid> {
    let mut tx = db.begin().await;
    ReferenceBase::newest_in(
        &mut tx,
        &BranchId::new(MAIN),
        &CurrencyCode::parse(currency).expect("currency"),
    )
    .await
    .expect("newest_in")
}

#[tokio::test]
async fn branch_anchor_bot_with_no_currency_setting_reads_bot_as_today() {
    let db = TestDb::start().await;
    branch_anchored(&db, "BOT").await;
    let bot_line = bot(&db, "USD", 2).await;
    tt(&db, "USD", 1).await; // newer, but a market line
    assert_eq!(
        base(&db, "USD").await,
        Some(bot_line),
        "§11: anchor BOT ⇒ AUTO prices exactly as today"
    );
}

#[tokio::test]
async fn a_fresh_market_batch_changes_nothing_while_the_anchor_is_bot() {
    let db = TestDb::start().await;
    branch_anchored(&db, "BOT").await;
    let bot_line = bot(&db, "USD", 3).await;
    tt(&db, "USD", 0).await;
    sr(&db, "USD", 0).await;
    assert_eq!(
        base(&db, "USD").await,
        Some(bot_line),
        "§4 [required]: a MARKET batch changes no AUTO price"
    );
}

#[tokio::test]
async fn currency_anchor_market_tt_reads_the_newest_tt_line() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "BOT").await;
    currency_anchored(&db, v1, "USD", Some("MARKET_TT")).await;
    bot(&db, "USD", 0).await;
    tt(&db, "USD", 3).await;
    let newest_tt = tt(&db, "USD", 1).await;
    assert_eq!(base(&db, "USD").await, Some(newest_tt));
}

#[tokio::test]
async fn the_currency_setting_applies_to_that_currency_only() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "BOT").await;
    currency_anchored(&db, v1, "USD", Some("MARKET_TT")).await;
    let eur_bot = bot(&db, "EUR", 2).await;
    tt(&db, "EUR", 1).await;
    assert_eq!(
        base(&db, "EUR").await,
        Some(eur_bot),
        "EUR has no setting ⇒ the branch's BOT"
    );
}

#[tokio::test]
async fn a_market_tt_anchor_with_no_tt_line_has_no_base_never_bot_or_start() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "BOT").await;
    currency_anchored(&db, v1, "USD", Some("MARKET_TT")).await;
    bot(&db, "USD", 1).await;
    start(&db, "USD", 2).await;
    tt(&db, "EUR", 1).await; // another currency's TT line
    assert_eq!(
        base(&db, "USD").await,
        None,
        "§3.1 [required]: no fallback to BoT or START"
    );
}

#[tokio::test]
async fn a_stale_tt_line_is_still_the_base() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "BOT").await;
    currency_anchored(&db, v1, "USD", Some("MARKET_TT")).await;
    let old_tt = tt(&db, "USD", 72).await;
    bot(&db, "USD", 0).await;
    assert_eq!(
        base(&db, "USD").await,
        Some(old_tt),
        "§3.1: staleness is StalePolicy's call at quote time"
    );
}

#[tokio::test]
async fn branch_anchor_market_tt_applies_to_currencies_without_a_setting() {
    let db = TestDb::start().await;
    branch_anchored(&db, "MARKET_TT").await;
    bot(&db, "USD", 0).await;
    let tt_line = tt(&db, "USD", 1).await;
    assert_eq!(base(&db, "USD").await, Some(tt_line));
}

#[tokio::test]
async fn a_currency_setting_of_bot_overrides_a_market_branch_anchor() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "MARKET_TT").await;
    currency_anchored(&db, v1, "USD", Some("BOT")).await;
    let bot_line = bot(&db, "USD", 2).await;
    tt(&db, "USD", 1).await;
    assert_eq!(base(&db, "USD").await, Some(bot_line));
}

#[tokio::test]
async fn a_settings_row_with_no_anchor_follows_the_branch() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "MARKET_SR").await;
    currency_anchored(&db, v1, "USD", None).await; // e.g. a min-spread-only row
    bot(&db, "USD", 0).await;
    let sr_line = sr(&db, "USD", 1).await;
    assert_eq!(base(&db, "USD").await, Some(sr_line));
}

#[tokio::test]
async fn the_bot_anchor_keeps_its_start_fallback() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "MARKET_TT").await;
    currency_anchored(&db, v1, "USD", Some("BOT")).await;
    let start_line = start(&db, "USD", 1).await;
    assert_eq!(
        base(&db, "USD").await,
        Some(start_line),
        "§3.1: START is the BOT anchor's fallback"
    );
}

#[tokio::test]
async fn a_market_sr_anchor_with_only_tt_lines_has_no_base() {
    let db = TestDb::start().await;
    branch_anchored(&db, "MARKET_SR").await;
    tt(&db, "USD", 1).await;
    bot(&db, "USD", 1).await;
    assert_eq!(base(&db, "USD").await, None);
}

#[tokio::test]
async fn only_the_current_policy_version_counts() {
    let db = TestDb::start().await;
    let v1 = branch_anchored(&db, "MARKET_TT").await;
    // Version 1 closed an hour ago; version 2 (BOT) is current.
    sqlx::query(
        "UPDATE rate_policy_versions SET valid_to = now() - interval '1 hour' WHERE id = $1",
    )
    .bind(v1)
    .execute(&db.pool)
    .await
    .expect("close v1");
    sqlx::query(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, valid_from, valid_to, \
            approved_by, reason, source_version, anchor_source) \
         VALUES (gen_random_uuid(), $1, 2, 0, 0, '{}'::jsonb, now() - interval '1 hour', NULL, 'owner-1', 'v2', 1, 'BOT')",
    )
    .bind(MAIN)
    .execute(&db.pool)
    .await
    .expect("insert v2");
    let bot_line = bot(&db, "USD", 2).await;
    tt(&db, "USD", 1).await;
    assert_eq!(base(&db, "USD").await, Some(bot_line));
}
