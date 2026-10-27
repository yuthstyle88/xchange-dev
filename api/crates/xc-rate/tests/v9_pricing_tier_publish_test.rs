//! XC-V9-PRICING PV-b2 (#5156): `TierSetPublisher::publish_in` (pricing-tiers.md §15.3 steps 4–6, §15.4 "publish
//! through steps 4–6", §15.5; behaviour PV-2 of §15.11). Publishing one tier set at its branch copies the branch's
//! current publication (an empty draft before the first one), replaces the currency's line with MANUAL tier 1 plus
//! its 4 tier rows, submits with the set's key and publishes, in the caller's transaction. Other currencies keep
//! their lines; the set row learns its `draft_id`; the same key never publishes twice.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `H:` is the owner's handoff (`PRICING-DESIGN.md`, kept out of the repo). PV-1's tiers are `H:` §4.4's.
//!
//! Readings:
//! - reading: a second `publish_in` with the same `submit_key` copies a new draft and submits it under a key whose
//!   `RATE_PUBLICATION` request already names the first draft. `ApprovalService::request` answers that with
//!   `IdempotencyConflict` (another object under the same key), which `RateDraftService::submit` passes on as
//!   `RateEngineError::IdempotencyConflict`; the caller rolls back, so nothing is published twice.
//! - reading: the 4 tier lines are coded `T1`..`T4` in V.9's bands (T1 (—, 10 000], T2 (10 000, 50 000],
//!   T3 (50 000, 200 000], T4 (200 000, —) THB, `H:` §4.1), as in `v9_pricing_draft_tiers_test.rs`.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test` (see
//! `rate_engine_common`). Every test fails without it; none skips.

mod rate_engine_common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_rate::application::pricing::tier_publish::TierSetPublisher;
use xc_rate::domain::draft::rate_draft::DraftLine;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::tier_set::{
    MarketAnchor, PricingTierSet, TierRate, TierSetSource, TierSetStore,
};
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

/// PV-1's tiers (`H:` §4.4), per unit size 1.
const PV1: [(&str, &str); 4] = [
    ("33.23", "33.87"),
    ("33.24", "33.86"),
    ("33.26", "33.84"),
    ("33.29", "33.80"),
];

fn dec(text: &str) -> Decimal {
    text.parse()
        .unwrap_or_else(|e| panic!("decimal {text}: {e}"))
}

fn v9_bands() -> Vec<(Option<Decimal>, Option<Decimal>)> {
    vec![
        (None, Some(dec("10000"))),
        (Some(dec("10000")), Some(dec("50000"))),
        (Some(dec("50000")), Some(dec("200000"))),
        (Some(dec("200000")), None),
    ]
}

fn whole_second_now() -> DateTime<Utc> {
    Utc.timestamp_opt(Utc::now().timestamp(), 0)
        .single()
        .expect("now")
}

fn guards() -> Value {
    let g = json!({ "buy": { "min_pct": "-5.0000", "max_pct": "5.0000" },
                    "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" } });
    json!({ "USD": g.clone(), "EUR": g })
}

/// MAIN's policy version 1 (adjust −0.5 / +0.4, guards ±5 %, value tiers off), a BoT batch an hour old, and a fresh
/// TT batch with its `market_feed_status`. Answers the TT `(batch id, fetched_at)`.
async fn main_room(db: &TestDb) -> (Uuid, DateTime<Utc>) {
    db.seed_policy(MAIN, "-0.5", "0.4", guards(), None, None, None)
        .await;
    db.seed_batch(
        Utc::now() - Duration::hours(1),
        &[
            ("USD", "1", Some("32.4500"), Some("32.9000")),
            ("EUR", "1", Some("38.0000"), Some("38.6000")),
        ],
    )
    .await;
    let at = whole_second_now();
    let batch: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), 'TT', CURRENT_DATE, $1, $2, 'COMPLETE') RETURNING id",
    )
    .bind(at)
    .bind(unique("chk"))
    .fetch_one(&db.pool)
    .await
    .expect("insert the TT batch");
    sqlx::query(
        "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1, 'USD', 'MARKET_TT', 1, 33.20, 33.90, '{}'::jsonb)",
    )
    .bind(batch)
    .execute(&db.pool)
    .await
    .expect("insert the TT line");
    sqlx::query(
        "INSERT INTO market_feed_status (source, last_attempt_at, last_error) VALUES ('TT', $1, NULL) \
         ON CONFLICT (source) DO UPDATE SET last_attempt_at = EXCLUDED.last_attempt_at, last_error = NULL",
    )
    .bind(at)
    .execute(&db.pool)
    .await
    .expect("upsert market_feed_status");
    (batch, at)
}

/// A MANUAL tier set of USD at MAIN with PV-1's tiers, anchored on TT.
fn usd_set(tt: (Uuid, DateTime<Utc>)) -> PricingTierSet {
    PricingTierSet {
        id: Uuid::new_v4(),
        request_id: format!("{}:USD", unique("approve")),
        branch: MAIN.to_owned(),
        currency: code("USD"),
        source: TierSetSource::Manual,
        base_tier_set_id: None,
        tiers: PV1.map(|(b, s)| TierRate {
            buy: rate(b),
            sell: rate(s),
        }),
        anchor: Some(MarketAnchor {
            source: MarketSource::Tt,
            buy: rate("33.20"),
            sell: rate("33.90"),
            fetched_at: tt.1,
            batch_id: Some(tt.0),
        }),
        flags: Vec::new(),
        draft_id: None,
        approved_by: OWNER_ID.to_owned(),
        approved_at: Utc::now(),
    }
}

/// Today's path, by the owner: a draft of `lines` at MAIN, submitted and published.
async fn owner_publishes(db: &TestDb, lines: Vec<DraftLine>, expected: Option<i32>) {
    let e = db.engine();
    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &owner_user(), lines)
        .await
        .expect("create");
    e.drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &draft.id, &key())
        .await
        .expect("submit");
    e.publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &draft.id, expected)
        .await
        .expect("publish");
    tx.commit().await.expect("commit today's publication");
}

fn publisher(db: &TestDb) -> TierSetPublisher {
    let e = db.engine();
    TierSetPublisher::new(e.drafts, e.publisher)
}

type PubLineRow = (
    String,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Value,
);

/// `(value_tier, lower, upper, buy, sell, source_snapshot)` of `currency` in MAIN's current publication.
async fn current_lines(db: &TestDb, currency: &str) -> Vec<PubLineRow> {
    sqlx::query_as(
        "SELECT l.value_tier, l.tier_lower_exclusive_thb, l.tier_upper_inclusive_thb, l.buy_rate, l.sell_rate, \
                l.source_snapshot \
         FROM published_rate_lines l JOIN branches b ON b.current_publication_id = l.publication_id \
         WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = 'ALL' \
         ORDER BY l.tier_lower_exclusive_thb NULLS FIRST, l.value_tier",
    )
    .bind(MAIN)
    .bind(currency)
    .fetch_all(&db.pool)
    .await
    .expect("read published_rate_lines")
}

/// Asserts USD's current lines are PV-1's 4 tiers in V.9's bands, naming `set`.
async fn assert_usd_published(db: &TestDb, set: &PricingTierSet) {
    let lines = current_lines(db, "USD").await;
    assert_eq!(
        lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        vec!["T1", "T2", "T3", "T4"],
        "§15.3 step 6: USD is 4 tier lines: {lines:#?}"
    );
    assert_eq!(
        lines.iter().map(|l| (l.1, l.2)).collect::<Vec<_>>(),
        v9_bands(),
        "V.9's bands"
    );
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.3.expect("buy"), l.4.expect("sell")))
            .collect::<Vec<_>>(),
        PV1.iter()
            .map(|(b, s)| (dec(b), dec(s)))
            .collect::<Vec<_>>(),
        "the set's prices exactly"
    );
    for l in &lines {
        assert_eq!(
            l.5["pricing"],
            json!({ "tier_set_id": set.id.to_string(), "source": "MANUAL" }),
            "§15.3 step 6: source_snapshot.pricing on {}",
            l.0
        );
    }
}

async fn draft_id_of_set(db: &TestDb, set: &PricingTierSet) -> Option<Uuid> {
    sqlx::query_scalar("SELECT draft_id FROM pricing_tier_sets WHERE id = $1")
        .bind(set.id)
        .fetch_one(&db.pool)
        .await
        .expect("read pricing_tier_sets.draft_id")
}

/// PV-2 (§15.3 steps 4–6): with MAIN publishing USD + EUR (AUTO), `publish_in` for a USD set publishes USD's 4
/// tiers, keeps EUR's line, records the draft on the set row, and answers the new publication. The caller is a
/// RATE_MANAGER: no EXECUTIVE step (B3).
#[tokio::test]
async fn publish_in_replaces_the_currency_and_keeps_the_other_lines() {
    let db = TestDb::start().await;
    let tt = main_room(&db).await;
    owner_publishes(&db, vec![auto_line("USD"), auto_line("EUR")], None).await;
    let eur_before = current_lines(&db, "EUR").await;
    assert_eq!(eur_before.len(), 1, "one AUTO EUR line: {eur_before:?}");

    let set = usd_set(tt);
    let mut tx = db.begin().await;
    assert!(
        PgTierSetStore
            .insert_in(&mut tx, &set)
            .await
            .expect("insert the set")
    );
    let out = publisher(&db)
        .publish_in(&mut tx, &rates_user(), &set, &set.request_id)
        .await
        .expect("publish_in");
    tx.commit().await.expect("commit");

    assert_eq!(
        db.current_publication(MAIN).await,
        Some(out.publication.id.to_string()),
        "the answer names MAIN's new current publication"
    );
    assert_eq!(out.publication.version, 2, "the second publication of MAIN");
    let draft_of_publication: Uuid =
        sqlx::query_scalar("SELECT draft_id FROM rate_publications WHERE id = $1")
            .bind(out.publication.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read rate_publications.draft_id");
    assert_eq!(
        draft_of_publication,
        out.draft_id.as_uuid(),
        "the answer names the draft it published"
    );
    assert_eq!(
        draft_id_of_set(&db, &set).await,
        Some(out.draft_id.as_uuid()),
        "§15.3 step 4: the set row's draft_id is the publishing draft"
    );
    assert_usd_published(&db, &set).await;
    let eur_after = current_lines(&db, "EUR").await;
    assert_eq!(
        eur_after
            .iter()
            .map(|l| (l.0.clone(), l.3, l.4))
            .collect::<Vec<_>>(),
        eur_before
            .iter()
            .map(|l| (l.0.clone(), l.3, l.4))
            .collect::<Vec<_>>(),
        "EUR keeps its line"
    );
    db.finish().await;
}

/// PV-2 (§15.3 step 4 "an empty draft before the first one"): before any publication at MAIN, `publish_in`
/// publishes a publication holding only that currency.
#[tokio::test]
async fn publish_in_before_any_publication_publishes_only_that_currency() {
    let db = TestDb::start().await;
    let tt = main_room(&db).await;
    assert_eq!(
        db.current_publication(MAIN).await,
        None,
        "no publication yet"
    );

    let set = usd_set(tt);
    let mut tx = db.begin().await;
    assert!(
        PgTierSetStore
            .insert_in(&mut tx, &set)
            .await
            .expect("insert the set")
    );
    let out = publisher(&db)
        .publish_in(&mut tx, &rates_user(), &set, &set.request_id)
        .await
        .expect("publish_in");
    tx.commit().await.expect("commit");

    assert_eq!(out.publication.version, 1);
    assert_eq!(
        db.publications(MAIN).await,
        vec![(1, out.publication.id.to_string())]
    );
    assert_usd_published(&db, &set).await;
    let currencies: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT currency_code::text FROM published_rate_lines WHERE publication_id = $1",
    )
    .bind(out.publication.id.as_uuid())
    .fetch_all(&db.pool)
    .await
    .expect("read the publication's currencies");
    assert_eq!(currencies, vec!["USD".to_owned()], "only USD");
    assert_eq!(
        draft_id_of_set(&db, &set).await,
        Some(out.draft_id.as_uuid())
    );
    db.finish().await;
}

/// PV-2 (§15.3 step 3 "keyed on requestId", §15.4 "Retrying with the same request id"): the same `submit_key` twice
/// does not publish twice. The second call answers `IdempotencyConflict` (see the reading) and, rolled back, leaves
/// MAIN's publications as they were.
#[tokio::test]
async fn publish_in_twice_with_the_same_submit_key_publishes_once() {
    let db = TestDb::start().await;
    let tt = main_room(&db).await;
    let set = usd_set(tt);

    let mut tx = db.begin().await;
    assert!(
        PgTierSetStore
            .insert_in(&mut tx, &set)
            .await
            .expect("insert the set")
    );
    let first = publisher(&db)
        .publish_in(&mut tx, &rates_user(), &set, &set.request_id)
        .await
        .expect("the first publish_in");
    tx.commit().await.expect("commit");
    let publications = db.publications(MAIN).await;
    assert_eq!(publications, vec![(1, first.publication.id.to_string())]);

    let mut tx = db.begin().await;
    let again = publisher(&db)
        .publish_in(&mut tx, &rates_user(), &set, &set.request_id)
        .await;
    tx.rollback().await.expect("the caller rolls back");
    assert!(
        matches!(again, Err(RateEngineError::IdempotencyConflict(_))),
        "the repeated key is refused by submit: {again:?}"
    );
    assert_eq!(
        db.publications(MAIN).await,
        publications,
        "nothing published twice"
    );
    assert_eq!(
        db.current_publication(MAIN).await,
        Some(first.publication.id.to_string())
    );
    assert_eq!(
        draft_id_of_set(&db, &set).await,
        Some(first.draft_id.as_uuid()),
        "the set still names the first draft"
    );
    db.finish().await;
}
