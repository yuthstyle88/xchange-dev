//! PT-4 (pricing-tiers.md §3 "Resolution", §3.1 snapshot `anchor_source`, §7 market snapshot, §11): the three
//! callers of `TierProposal::of` (`rate_draft_service.rs`, `draft_evaluation.rs`, `publish_service.rs`) price each
//! currency with `PolicyTiers::for_currency(<its override>)`, and every published line's `source_snapshot` gains
//! `"market"` and `"anchor_source"`.
//!
//! Written before the implementation by the ticket author (claude/647e1967, PT-0). Do not edit, delete or weaken;
//! add tests in a file of your own. Override rows are seeded by SQL; the implementation reads them through
//! `CurrencyTierOverrideStore` (PT-3) and resolves with `for_currency` (PT-2), so these pass once PT-2 and PT-3
//! are on `feat/pricing-tiers`; the `MARKET_TT` snapshot test also needs PT-15.
//!
//! Snapshot shape pinned here (§7): `"market"` is an array, newest `MARKET_TT` line first, then the newest `MARKET_SR`
//! line, each `{"source": <reference_type>, "reference_line_id", "unit_size", "buy", "sell", "fetched_at"}` with
//! decimals as normalized text (like `"partner"`), `[]` when the currency has no market line. `"anchor_source"` is
//! the `reference_type` of the line `reference_line_id` points at, `null` when there is none.
//!
//! Prices: §12.3's worked example (USD BoT 32.4500 / 32.9000, adjust −0.5 / +0.4); a tier price is
//! base × (1 + (adjust + margin) ÷ 100) at 8 places.
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database ending in `_test`.

mod rate_engine_common;

use chrono::{DateTime, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;
use xc_auth::domain::access::{self, Actor, ScopedRole};
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, LineSide};

fn owner_actor() -> Actor {
    Actor::new(
        OWNER_ID,
        ORG,
        vec![
            ScopedRole::new(
                access::Role::Executive,
                None,
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            ),
            ScopedRole::new(access::Role::Admin, None, DateTime::<Utc>::UNIX_EPOCH, None),
        ],
        None,
    )
}

fn guards() -> Value {
    let g = json!({ "buy": { "min_pct": "-5.0000", "max_pct": "5.0000" },
                    "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" } });
    json!({ "USD": g.clone(), "EUR": g })
}

/// Version 1 of `MAIN` with §12.3's bands and margins (T1 0/0, T2 +0.1/−0.1, T3 +0.2/−0.2, T4 +0.3/−0.3).
async fn policy(db: &TestDb, value_tiers_on: bool) -> Uuid {
    db.seed_branch(MAIN).await;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, valid_from, valid_to, \
            approved_by, reason, value_tiers_on) \
         VALUES (gen_random_uuid(), $1, 1, -0.5, 0.4, $2, now() - interval '1 day', NULL, $3, 'seeded', $4) \
         RETURNING id",
    )
    .bind(MAIN)
    .bind(guards())
    .bind(OWNER_ID)
    .bind(value_tiers_on)
    .fetch_one(&db.pool)
    .await
    .expect("insert rate_policy_versions");
    for (no, upper, buy, sell) in [
        (1i16, Some("20000.00"), "0.0000", "0.0000"),
        (2, Some("100000.00"), "0.1000", "-0.1000"),
        (3, Some("500000.00"), "0.2000", "-0.2000"),
        (4, None, "0.3000", "-0.3000"),
    ] {
        sqlx::query(
            "INSERT INTO rate_policy_value_tiers (policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct) \
             VALUES ($1, $2, $3::numeric, $4::numeric, $5::numeric)",
        )
        .bind(id)
        .bind(no)
        .bind(upper)
        .bind(buy)
        .bind(sell)
        .execute(&db.pool)
        .await
        .expect("insert rate_policy_value_tiers");
    }
    id
}

/// `currency`'s four override margins on `version` (`rate_policy_currency_tiers`, §3).
async fn override_margins(db: &TestDb, version: Uuid, currency: &str, margins: [(&str, &str); 4]) {
    for (i, (buy, sell)) in margins.iter().enumerate() {
        sqlx::query(
            "INSERT INTO rate_policy_currency_tiers (policy_version_id, currency_code, tier_no, buy_margin_pct, sell_margin_pct) \
             VALUES ($1, $2, $3, $4::numeric, $5::numeric)",
        )
        .bind(version)
        .bind(currency)
        .bind((i + 1) as i16)
        .bind(*buy)
        .bind(*sell)
        .execute(&db.pool)
        .await
        .expect("insert rate_policy_currency_tiers");
    }
}

const USD_OVERRIDE: [(&str, &str); 4] = [
    ("0.4000", "-0.4000"),
    ("0.4500", "-0.4500"),
    ("0.5000", "-0.5000"),
    ("0.6000", "-0.6000"),
];

/// One market batch (`provider`) with one `reference_type` line for `currency`.
async fn market_line(
    db: &TestDb,
    currency: &str,
    reference_type: &str,
    provider: &str,
    buy: &str,
    sell: &str,
    hours_ago: i64,
) -> Uuid {
    let batch: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), $1, CURRENT_DATE, now() - make_interval(hours => $2::int), $3, 'COMPLETE') RETURNING id",
    )
    .bind(provider)
    .bind(hours_ago as i32)
    .bind(unique("chk"))
    .fetch_one(&db.pool)
    .await
    .expect("insert market batch");
    sqlx::query_scalar(
        "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1, $2, $3, 1, $4::numeric, $5::numeric, '{}'::jsonb) RETURNING id",
    )
    .bind(batch)
    .bind(currency)
    .bind(reference_type)
    .bind(buy)
    .bind(sell)
    .fetch_one(&db.pool)
    .await
    .expect("insert market line")
}

async fn bot_batch(db: &TestDb) {
    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[
            ("USD", "1", Some("32.4500"), Some("32.9000")),
            ("EUR", "1", Some("38.0000"), Some("38.6000")),
        ],
    )
    .await;
}

/// Creates, submits and (when approved) publishes an all-AUTO draft for `currencies`; answers the submit status
/// and the publication id.
async fn run_draft(db: &TestDb, currencies: &[&str]) -> (DraftStatus, Option<String>) {
    let e = db.engine();
    let who = owner_actor();
    let lines: Vec<DraftLine> = currencies
        .iter()
        .map(|c| DraftLine::new(code(c), LineSide::Auto, LineSide::Auto, None))
        .collect();
    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &who, lines)
        .await
        .expect("create draft");
    tx.commit().await.expect("commit create");
    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &who, &draft.id, &key())
        .await
        .expect("submit");
    tx.commit().await.expect("commit submit");
    if outcome.status != DraftStatus::Approved {
        return (outcome.status, None);
    }
    let mut tx = db.begin().await;
    let published = e
        .publisher
        .publish(&mut tx, &main_branch(), &who, &draft.id, None)
        .await
        .expect("publish");
    tx.commit().await.expect("commit publish");
    (outcome.status, Some(published.id.to_string()))
}

/// `(value_tier, buy_rate, sell_rate, source_snapshot)` of `currency`'s published lines, by tier.
async fn lines_of(
    db: &TestDb,
    publication: &str,
    currency: &str,
) -> Vec<(String, String, String, Value)> {
    sqlx::query(
        "SELECT value_tier, buy_rate, sell_rate, source_snapshot FROM published_rate_lines \
         WHERE publication_id = $1::uuid AND currency_code = $2 ORDER BY value_tier",
    )
    .bind(publication)
    .bind(currency)
    .fetch_all(&db.pool)
    .await
    .expect("published lines")
    .iter()
    .map(|r| {
        (
            r.get::<String, _>(0),
            r.get::<Decimal, _>(1).to_string(),
            r.get::<Decimal, _>(2).to_string(),
            r.get::<Value, _>(3),
        )
    })
    .collect()
}

fn rates(lines: &[(String, String, String, Value)]) -> Vec<(&str, &str, &str)> {
    lines
        .iter()
        .map(|(t, b, s, _)| (t.as_str(), b.as_str(), s.as_str()))
        .collect()
}

#[tokio::test]
async fn tiers_on_a_currency_with_an_override_prices_with_its_margins_and_the_others_do_not_change()
{
    let db = TestDb::start().await;
    bot_batch(&db).await;
    let v1 = policy(&db, true).await;
    override_margins(&db, v1, "USD", USD_OVERRIDE).await;

    let (status, publication) = run_draft(&db, &["USD", "EUR"]).await;
    assert_eq!(status, DraftStatus::Approved);
    let publication = publication.expect("published");

    assert_eq!(
        rates(&lines_of(&db, &publication, "USD").await),
        vec![
            ("T1", "32.41755000", "32.90000000"),
            ("T2", "32.43377500", "32.88355000"),
            ("T3", "32.45000000", "32.86710000"),
            ("T4", "32.48245000", "32.83420000"),
        ],
        "§11: that currency's 4 lines use its margins"
    );
    assert_eq!(
        rates(&lines_of(&db, &publication, "EUR").await),
        vec![
            ("T1", "37.81000000", "38.75440000"),
            ("T2", "37.84800000", "38.71580000"),
            ("T3", "37.88600000", "38.67720000"),
            ("T4", "37.92400000", "38.63860000"),
        ],
        "§11: other currencies unchanged (the branch's margins)"
    );
}

#[tokio::test]
async fn each_tier_snapshot_carries_the_margins_actually_used() {
    let db = TestDb::start().await;
    bot_batch(&db).await;
    let v1 = policy(&db, true).await;
    override_margins(&db, v1, "USD", USD_OVERRIDE).await;
    let (_, publication) = run_draft(&db, &["USD"]).await;
    let lines = lines_of(&db, &publication.expect("published"), "USD").await;
    let t2 = &lines[1].3["tier"];
    assert_eq!(t2["no"], json!(2));
    assert_eq!(t2["buy_margin_pct"], json!("0.4500"));
    assert_eq!(t2["sell_margin_pct"], json!("-0.4500"));
}

#[tokio::test]
async fn tiers_off_ignores_override_rows_and_keeps_them() {
    let db = TestDb::start().await;
    bot_batch(&db).await;
    let v1 = policy(&db, false).await;
    override_margins(&db, v1, "USD", USD_OVERRIDE).await;

    let (status, publication) = run_draft(&db, &["USD"]).await;
    assert_eq!(status, DraftStatus::Approved);
    assert_eq!(
        rates(&lines_of(&db, &publication.expect("published"), "USD").await),
        vec![("ALL", "32.28775000", "33.03160000")],
        "§11: tiers OFF ⇒ one 'ALL' line, overrides ignored"
    );
    assert_eq!(db.count("rate_policy_currency_tiers").await, 4, "rows kept");
}

#[tokio::test]
async fn an_override_margin_outside_the_guard_makes_the_draft_out_of_bounds() {
    let db = TestDb::start().await;
    bot_batch(&db).await;
    let v1 = policy(&db, true).await;
    // T4 buy: −0.5 + 6.0 = 5.5 > the USD guard's 5.0; the branch's own margins are all inside.
    override_margins(
        &db,
        v1,
        "USD",
        [
            ("0.0000", "0.0000"),
            ("0.1000", "-0.1000"),
            ("0.2000", "-0.2000"),
            ("6.0000", "-0.3000"),
        ],
    )
    .await;
    let (status, publication) = run_draft(&db, &["USD"]).await;
    assert_eq!(
        status,
        DraftStatus::PendingApproval,
        "draft_evaluation checks the resolved tiers"
    );
    assert!(publication.is_none());
}

#[tokio::test]
async fn the_snapshot_holds_the_newest_market_lines_of_the_currency() {
    let db = TestDb::start().await;
    bot_batch(&db).await;
    policy(&db, false).await;
    market_line(&db, "USD", "MARKET_TT", "TT", "32.0000", "33.5000", 3).await;
    let tt = market_line(&db, "USD", "MARKET_TT", "TT", "32.5000", "33.1000", 1).await;
    let sr = market_line(&db, "USD", "MARKET_SR", "SR", "32.6000", "33.0000", 2).await;
    market_line(&db, "EUR", "MARKET_TT", "TT", "38.1000", "38.5000", 1).await;

    let (_, publication) = run_draft(&db, &["USD"]).await;
    let lines = lines_of(&db, &publication.expect("published"), "USD").await;
    let market = &lines[0].3["market"];
    let market = market
        .as_array()
        .unwrap_or_else(|| panic!("source_snapshot.market is an array: {market}"));
    assert_eq!(
        market.len(),
        2,
        "one newest line per market type: {market:?}"
    );

    assert_eq!(market[0]["source"], json!("MARKET_TT"));
    assert_eq!(market[0]["reference_line_id"], json!(tt.to_string()));
    assert_eq!(market[0]["buy"], json!("32.5"));
    assert_eq!(market[0]["sell"], json!("33.1"));
    assert_eq!(market[0]["unit_size"], json!("1"));
    assert!(market[0]["fetched_at"].is_string());

    assert_eq!(market[1]["source"], json!("MARKET_SR"));
    assert_eq!(market[1]["reference_line_id"], json!(sr.to_string()));
    assert_eq!(market[1]["buy"], json!("32.6"));
    assert_eq!(market[1]["sell"], json!("33"));
}

#[tokio::test]
async fn a_currency_with_no_market_line_snapshots_an_empty_market() {
    let db = TestDb::start().await;
    bot_batch(&db).await;
    policy(&db, true).await;
    market_line(&db, "EUR", "MARKET_TT", "TT", "38.1000", "38.5000", 1).await;
    let (_, publication) = run_draft(&db, &["USD"]).await;
    for (tier, _, _, snapshot) in lines_of(&db, &publication.expect("published"), "USD").await {
        assert_eq!(
            snapshot["market"],
            json!([]),
            "{tier}: §7 a currency with no market line writes []"
        );
    }
}

#[tokio::test]
async fn the_snapshot_names_bot_as_the_anchor_of_a_bot_priced_line() {
    let db = TestDb::start().await;
    bot_batch(&db).await;
    policy(&db, true).await;
    market_line(&db, "USD", "MARKET_TT", "TT", "32.5000", "33.1000", 0).await;
    let (_, publication) = run_draft(&db, &["USD"]).await;
    for (tier, _, _, snapshot) in lines_of(&db, &publication.expect("published"), "USD").await {
        assert_eq!(snapshot["anchor_source"], json!("BOT"), "{tier}");
    }
}

#[tokio::test]
async fn the_snapshot_names_market_tt_when_the_currency_is_anchored_on_tt() {
    let db = TestDb::start().await;
    bot_batch(&db).await;
    let v1 = policy(&db, false).await;
    sqlx::query(
        "INSERT INTO rate_policy_currency_settings (policy_version_id, currency_code, anchor_source) VALUES ($1, 'USD', 'MARKET_TT')",
    )
    .bind(v1)
    .execute(&db.pool)
    .await
    .expect("anchor USD on TT");
    let tt = market_line(&db, "USD", "MARKET_TT", "TT", "32.5000", "33.1000", 0).await;

    let (status, publication) = run_draft(&db, &["USD"]).await;
    assert_eq!(status, DraftStatus::Approved);
    let lines = lines_of(&db, &publication.expect("published"), "USD").await;
    let snapshot = &lines[0].3;
    assert_eq!(
        snapshot["reference_line_id"],
        json!(tt.to_string()),
        "PT-15: the TT line is the AUTO base"
    );
    assert_eq!(
        snapshot["anchor_source"],
        json!("MARKET_TT"),
        "§11: snapshot anchor_source = 'MARKET_TT'"
    );
    assert_eq!(
        rates(&lines),
        vec![("ALL", "32.33750000", "33.23240000")],
        "§11: that currency's AUTO price uses the TT line (32.5 × 0.995, 33.1 × 1.004)"
    );
}
