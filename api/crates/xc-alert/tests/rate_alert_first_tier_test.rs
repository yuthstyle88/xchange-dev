//! Up-front tests for R1 (#564 issuecomment-6542427118): rate-alert create and resume observe the branch's
//! current publication at its **first tier**, the same line the evaluator compares (`evaluator.rs:77-79`,
//! `tier_lower_exclusive_thb IS NULL`) and `my_rate_alerts` shows (R-10, `rate-feed-connector.md` §9.1).
//!
//! Why: a MANUAL approve publishes tier lines only (`pricing-tiers.md` §15.3 step 6, `:908`). Create and resume read
//! through `PublishedRates::line_in`, which reads only a `value_tier = 'ALL'` line, so on such a publication create
//! refuses "branch MAIN does not publish USD Sell" (the xc-v3 e2e failure of 2026-10-31,
//! issuecomment-6542359058) and resume observes nothing.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add
//! tests in a file of your own. Needs `TEST_DATABASE_URL` (see `common/mod.rs`).
//!
//! The tier figures are MAIN's v10 on xc-v3: T1 BUY 33.50 / SELL 34.22 … T4 BUY 33.57 / SELL 34.15. Each target
//! sits between T1 and the other tiers, so reading any other tier gives the other relation.

mod common;

use common::*;
use serde_json::json;

/// Publication `version` of MAIN with USD as four MANUAL tier lines (V.9 bands) and no `ALL` line.
async fn publish_usd_tiers(db: &TestDb, version: i32, tiers: [(&str, &str); 4]) -> uuid::Uuid {
    let publication = db.publish(MAIN, version, vec![]).await;
    let bands = [
        ("T1", "NULL", "10000"),
        ("T2", "10000", "50000"),
        ("T3", "50000", "200000"),
        ("T4", "200000", "NULL"),
    ];
    let snapshot = json!({ "buy_mode": "MANUAL", "sell_mode": "MANUAL" });
    for ((tier, lower, upper), (buy, sell)) in bands.iter().zip(tiers) {
        db.exec(&format!(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot, value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
             base_buy_rate, base_sell_rate) \
             VALUES (gen_random_uuid(), '{publication}', 'USD', 'ALL', 1, {buy}, {sell}, '{snapshot}'::jsonb, \
             '{tier}', {lower}, {upper}, 33.50, 34.22)"
        ))
        .await;
    }
    publication
}

const V10: [(&str, &str); 4] = [
    ("33.50", "34.22"),
    ("33.51", "34.21"),
    ("33.53", "34.19"),
    ("33.57", "34.15"),
];

/// Row 1: CUSTOMER_BUY (shop SELL) ABOVE 34.20 on a tier-only publication is created and observed at T1's
/// SELL 34.22: SATISFIED. T2–T4 (34.21 … 34.15) would give NOT_SATISFIED for T4, refusal on main.
#[tokio::test]
async fn create_on_a_tier_only_publication_observes_tier_1_sell() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    publish_usd_tiers(&db, 2, V10).await;
    let svc = db.service();
    let alert = db
        .create(
            &svc,
            CX1,
            &input(
                "USD",
                CustomerSide::CustomerBuy,
                AlertCondition::Above,
                "34.20",
            ),
            &key(),
        )
        .await
        .expect(
            "row 1: created on a tier-only publication (main refuses: does not publish USD Sell)",
        );
    let row = db.row(alert.id).await;
    assert_eq!(
        row.relation, "SATISFIED",
        "row 1: observed at T1's SELL 34.22"
    );
    assert_eq!(
        row.last_publication_version,
        Some(2),
        "row 1: the tier-only publication v2"
    );
    db.finish().await;
}

/// Row 2: CUSTOMER_SELL (shop BUY) BELOW 33.52 is observed at T1's BUY 33.50: SATISFIED (T3 33.53 and T4 33.57
/// would be NOT_SATISFIED).
#[tokio::test]
async fn create_on_a_tier_only_publication_observes_tier_1_buy() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    publish_usd_tiers(&db, 2, V10).await;
    let svc = db.service();
    let alert = db
        .create(
            &svc,
            CX1,
            &input(
                "USD",
                CustomerSide::CustomerSell,
                AlertCondition::Below,
                "33.52",
            ),
            &key(),
        )
        .await
        .expect(
            "row 2: created on a tier-only publication (main refuses: does not publish USD Buy)",
        );
    assert_eq!(
        db.row(alert.id).await.relation,
        "SATISFIED",
        "row 2: observed at T1's BUY 33.50"
    );
    db.finish().await;
}

/// Row 3: resume after the branch moved to a tier-only publication observes T1, not UNKNOWN.
#[tokio::test]
async fn resume_on_a_tier_only_publication_observes_tier_1() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    // v1 (world): USD SELL 34.00, under 35: NOT_SATISFIED.
    let alert = db
        .create(
            &svc,
            CX1,
            &input(
                "USD",
                CustomerSide::CustomerBuy,
                AlertCondition::Above,
                "35",
            ),
            &key(),
        )
        .await
        .expect("created on the untiered v1");
    let mut tx = db.begin().await;
    let paused = svc.pause(&mut tx, CX1, alert.id, 1, t0()).await;
    settle(tx, paused).await.expect("paused");

    // While paused, MAIN moves to a tier-only publication whose T1 SELL 35.10 is above 35 and T4 34.90 below.
    publish_usd_tiers(
        &db,
        2,
        [
            ("34.60", "35.10"),
            ("34.62", "35.05"),
            ("34.64", "34.98"),
            ("34.66", "34.90"),
        ],
    )
    .await;

    let mut tx = db.begin().await;
    let resumed = svc.resume(&mut tx, CX1, alert.id, 2, t0()).await;
    settle(tx, resumed).await.expect("resumed");
    let row = db.row(alert.id).await;
    assert_eq!(
        row.relation, "SATISFIED",
        "row 3: resume observes T1's SELL 35.10 (main: UNKNOWN, no ALL line)"
    );
    assert_eq!(row.last_publication_version, Some(2));
    assert_eq!(db.events().await, 0, "resume never fires");
    db.finish().await;
}

/// Row 4 (kept): a tier-only publication with no line for the currency still refuses `not_publishing`.
#[tokio::test]
async fn a_currency_the_tier_only_publication_lacks_is_still_refused() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    publish_usd_tiers(&db, 2, V10).await;
    let svc = db.service();
    let (code, _) = refusal(
        db.create(
            &svc,
            CX1,
            &input(
                "JPY",
                CustomerSide::CustomerBuy,
                AlertCondition::Above,
                "25",
            ),
            &key(),
        )
        .await,
    );
    assert_eq!(code, "validationFailed", "row 4: v2 has no JPY line");
    db.finish().await;
}
