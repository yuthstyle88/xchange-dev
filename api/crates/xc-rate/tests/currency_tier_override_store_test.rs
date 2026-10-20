//! PT-3 (pricing-tiers.md §3, §3.1, §10.1, §12): `CurrencyTierOverrideStore` over `rate_policy_currency_tiers` and
//! `rate_policy_currency_settings`. Written before the implementation by the ticket author (claude/647e1967, PT-0).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test` (see
//! `rate_engine_common`). Every test fails without it; none skips.

mod rate_engine_common;

use rate_engine_common::{MAIN, TestDb};
use serde_json::json;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, RatePercent, Side};
use xc_rate::domain::policy::anchor_source::AnchorSource;
use xc_rate::domain::policy::currency_tier_override::{CurrencyTierOverride, TierMargin};
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;

fn pct(text: &str) -> RatePercent {
    RatePercent::parse(text).expect("a percent")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency")
}

/// Built by hand (not through `CurrencyTierOverride::new`, PT-2's) so this file tests the store alone.
fn with_tiers(
    currency: &str,
    base: &str,
    min_spread: Option<&str>,
    anchor: Option<AnchorSource>,
) -> CurrencyTierOverride {
    let b: rust_decimal::Decimal = base.parse().expect("decimal");
    let tiers = [1u8, 2, 3, 4].map(|n| {
        let step = rust_decimal::Decimal::new(i64::from(n), 1); // 0.1 · n
        TierMargin {
            tier_no: n,
            buy_margin_pct: pct(&format!("{:.4}", -(b - step))),
            sell_margin_pct: pct(&format!("{:.4}", b - step)),
        }
    });
    CurrencyTierOverride {
        currency: code(currency),
        tiers: Some(tiers),
        min_spread_pct: min_spread.map(pct),
        anchor_source: anchor,
    }
}

fn settings_only(
    currency: &str,
    min_spread: Option<&str>,
    anchor: Option<AnchorSource>,
) -> CurrencyTierOverride {
    CurrencyTierOverride {
        currency: code(currency),
        tiers: None,
        min_spread_pct: min_spread.map(pct),
        anchor_source: anchor,
    }
}

async fn version_1(db: &TestDb) -> Uuid {
    db.seed_branch(MAIN).await;
    let id = db
        .seed_policy(MAIN, "0", "0", json!({}), None, None, None)
        .await;
    Uuid::parse_str(&id).expect("uuid")
}

/// Version 2 of `MAIN`, sourced from version 1, as `PolicyVersionWriter::next` would write it.
async fn version_2(db: &TestDb) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, valid_from, valid_to, \
            approved_by, reason, source_version) \
         VALUES (gen_random_uuid(), $1, 2, 0, 0, '{}'::jsonb, now(), NULL, 'owner-1', 'v2', 1) \
         RETURNING id",
    )
    .bind(MAIN)
    .fetch_one(&db.pool)
    .await
    .expect("insert version 2")
}

async fn count_on(db: &TestDb, table: &str, version: &Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE policy_version_id = $1"
    )))
    .bind(version)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("count {table}: {e}"))
}

#[tokio::test]
async fn a_version_with_no_override_rows_reads_empty() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let mut tx = db.begin().await;
    let got = CurrencyTierOverrideStore::for_version(&mut tx, &v1)
        .await
        .expect("read");
    assert!(
        got.is_empty(),
        "§12: every existing version has zero override rows: {got:?}"
    );
}

#[tokio::test]
async fn insert_then_for_version_reads_every_override_back_ordered_by_currency() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let usd = with_tiers("USD", "0.5", Some("1.2500"), Some(AnchorSource::MarketTt));
    let eur = with_tiers("EUR", "0.9", None, None);
    let jpy = settings_only("JPY", Some("0.7500"), None);
    let gbp = settings_only("GBP", None, Some(AnchorSource::MarketSr));

    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::insert(
        &mut tx,
        &v1,
        &[usd.clone(), jpy.clone(), eur.clone(), gbp.clone()],
    )
    .await
    .expect("insert");
    let got = CurrencyTierOverrideStore::for_version(&mut tx, &v1)
        .await
        .expect("read");
    tx.commit().await.expect("commit");

    assert_eq!(
        got,
        vec![eur, gbp, jpy, usd],
        "ordered by currency code, every field round-trips"
    );
}

#[tokio::test]
async fn tier_rows_are_four_per_margin_override_and_settings_rows_only_when_there_is_a_setting() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::insert(
        &mut tx,
        &v1,
        &[
            with_tiers("USD", "0.5", Some("1.0000"), None), // 4 tier rows + 1 settings row
            with_tiers("EUR", "0.5", None, None),           // 4 tier rows, no settings row
            settings_only("JPY", None, Some(AnchorSource::Bot)), // settings row only
        ],
    )
    .await
    .expect("insert");
    tx.commit().await.expect("commit");

    assert_eq!(count_on(&db, "rate_policy_currency_tiers", &v1).await, 8);
    assert_eq!(count_on(&db, "rate_policy_currency_settings", &v1).await, 2);
    let anchor: Option<String> = sqlx::query_scalar(
        "SELECT anchor_source FROM rate_policy_currency_settings WHERE policy_version_id = $1 AND currency_code = 'JPY'",
    )
    .bind(v1)
    .fetch_one(&db.pool)
    .await
    .expect("JPY settings row");
    assert_eq!(
        anchor.as_deref(),
        Some("BOT"),
        "the stored text is AnchorSource::as_str"
    );
}

#[tokio::test]
async fn a_currency_with_a_settings_row_and_no_tier_rows_has_no_tiers() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::insert(&mut tx, &v1, &[settings_only("USD", Some("2.0000"), None)])
        .await
        .expect("insert");
    let got = CurrencyTierOverrideStore::for_version(&mut tx, &v1)
        .await
        .expect("read");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].tiers, None);
    assert_eq!(got[0].margin(1, Side::Buy), None);
    assert_eq!(got[0].min_spread_pct, Some(pct("2.0000")));
}

#[tokio::test]
async fn carry_forward_copies_every_override_onto_the_new_version_and_keeps_the_old() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let originals = vec![
        with_tiers("EUR", "0.9", None, Some(AnchorSource::MarketTt)),
        settings_only("JPY", Some("0.5000"), None),
        with_tiers("USD", "0.5", Some("1.0000"), None),
    ];
    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::insert(&mut tx, &v1, &originals)
        .await
        .expect("insert");
    tx.commit().await.expect("commit");

    let v2 = version_2(&db).await;
    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::carry_forward(&mut tx, &v1, &v2, &[])
        .await
        .expect("carry");
    let on_v2 = CurrencyTierOverrideStore::for_version(&mut tx, &v2)
        .await
        .expect("read v2");
    let on_v1 = CurrencyTierOverrideStore::for_version(&mut tx, &v1)
        .await
        .expect("read v1");
    tx.commit().await.expect("commit");

    assert_eq!(
        on_v2, originals,
        "§11: an adjust-only save carries every currency's overrides"
    );
    assert_eq!(
        on_v1, originals,
        "rows are immutable: the source version keeps its own"
    );
}

#[tokio::test]
async fn carry_forward_skips_the_currencies_in_except() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::insert(
        &mut tx,
        &v1,
        &[
            with_tiers("EUR", "0.9", Some("1.0000"), None),
            with_tiers("USD", "0.5", Some("1.0000"), Some(AnchorSource::MarketTt)),
        ],
    )
    .await
    .expect("insert");
    tx.commit().await.expect("commit");

    let v2 = version_2(&db).await;
    let replaced_usd = with_tiers("USD", "0.7", None, None);
    let mut tx = db.begin().await;
    // What PolicyVersionWriter::next does for `overrides: Some([USD])`: insert the replacement, carry the rest.
    CurrencyTierOverrideStore::insert(&mut tx, &v2, std::slice::from_ref(&replaced_usd))
        .await
        .expect("insert v2");
    CurrencyTierOverrideStore::carry_forward(&mut tx, &v1, &v2, &[code("USD")])
        .await
        .expect("carry except USD");
    let on_v2 = CurrencyTierOverrideStore::for_version(&mut tx, &v2)
        .await
        .expect("read v2");
    tx.commit().await.expect("commit");

    assert_eq!(
        on_v2,
        vec![with_tiers("EUR", "0.9", Some("1.0000"), None), replaced_usd]
    );
    assert_eq!(
        count_on(&db, "rate_policy_currency_settings", &v2).await,
        1,
        "USD's old settings row is not copied"
    );
}

#[tokio::test]
async fn carry_forward_from_a_version_with_nothing_writes_nothing() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let v2 = version_2(&db).await;
    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::carry_forward(&mut tx, &v1, &v2, &[])
        .await
        .expect("carry");
    tx.commit().await.expect("commit");
    assert_eq!(count_on(&db, "rate_policy_currency_tiers", &v2).await, 0);
    assert_eq!(count_on(&db, "rate_policy_currency_settings", &v2).await, 0);
}

#[tokio::test]
async fn the_store_never_commits_the_callers_transaction() {
    let db = TestDb::start().await;
    let v1 = version_1(&db).await;
    let mut tx = db.begin().await;
    CurrencyTierOverrideStore::insert(
        &mut tx,
        &v1,
        &[with_tiers("USD", "0.5", Some("1.0000"), None)],
    )
    .await
    .expect("insert");
    tx.rollback().await.expect("rollback");
    assert_eq!(count_on(&db, "rate_policy_currency_tiers", &v1).await, 0);
    assert_eq!(count_on(&db, "rate_policy_currency_settings", &v1).await, 0);
}
