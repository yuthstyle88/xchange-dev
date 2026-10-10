//! APP-4 (#994) — `PlannerPreview::estimate`: a fresh estimate from the branch's current
//! publication that writes nothing, and answers `stale` / `rateUnavailable` like `QuoteService`.
//! Written before the implementation by the ticket author's delegate (claude/dell-claude-3). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/customer-app.md` P4 (the customer side becomes the shop side only
//! through `CustomerSide::to_shop`: `CUSTOMER_BUY` is priced at the shop's sell rate), P5 (the
//! preview writes no quote, stock or booking), §4.3 (`PublishedRates::current`/`line_in`,
//! `Freshness`, `StalePolicy`), §9.1 row APP-4 (row counts before/after; JPY per 100). Rates design
//! §4.8: a line is stale at read time (a reference fetched 30 h ago against 24 h); the stale
//! policy is the one the line was published under (STOP refuses, LIMIT refuses above
//! `stale_limit_thb`, NULL admits). `api/tests/app_planner_quote_parity_test.rs` compares the
//! same cases with `QuoteService::quote_in` itself.
//!
//! Signatures this test assumes (the scaffold commit before this one): `PlannerPreview::new(pool)`;
//! `estimate(&self, tx, plan: &DraftPlan, now) -> Result<PlannerEstimate, CustomerAppError>`
//! (async); `PlannerEstimate { thb: Money, rate: Rate, unit_size: UnitSize, stale: bool,
//! source_date: Option<NaiveDate> }`.

mod common;

use chrono::Utc;
use common::*;
use xc_kernel::money::CustomerSide::{CustomerBuy, CustomerSell};
use xc_kernel::money::{Rate, UnitSize};

async fn estimate(db: &TestDb, plan: &DraftPlan) -> Result<PlannerEstimate, CustomerAppError> {
    let preview = PlannerPreview::new(db.pool.clone());
    let mut tx = db.begin().await;
    let answer = preview.estimate(&mut tx, plan, Utc::now()).await;
    tx.commit().await.expect("commit the preview's transaction");
    answer
}

#[tokio::test]
async fn customer_buy_is_estimated_at_the_shops_sell_rate() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;

    let answer = estimate(&db, &plan("USD", CustomerBuy, "100"))
        .await
        .expect("an estimate");

    assert_eq!(answer.rate, Rate::parse("32.45").expect("rate"));
    assert_eq!(answer.thb, money("3245.00"));
    assert_eq!(answer.unit_size, UnitSize::parse("1").expect("unit"));
    assert!(!answer.stale, "a MANUAL line is never stale");
    db.finish().await;
}

#[tokio::test]
async fn customer_sell_is_estimated_at_the_shops_buy_rate() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;

    let answer = estimate(&db, &plan("USD", CustomerSell, "100"))
        .await
        .expect("an estimate");

    assert_eq!(answer.rate, Rate::parse("32.10").expect("rate"));
    assert_eq!(answer.thb, money("3210.00"));
    db.finish().await;
}

#[tokio::test]
async fn jpy_10000_against_a_jpy_line_of_unit_100_is_priced_per_100() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[JPY]).await;

    let sell = estimate(&db, &plan("JPY", CustomerSell, "10000"))
        .await
        .expect("an estimate");
    let buy = estimate(&db, &plan("JPY", CustomerBuy, "10000"))
        .await
        .expect("an estimate");

    assert_eq!(sell.thb, money("2350.00"), "10 000 / 100 × 23.5");
    assert_eq!(buy.thb, money("2400.00"), "10 000 / 100 × 24.0");
    assert_eq!(sell.unit_size, UnitSize::parse("100").expect("unit"));
    db.finish().await;
}

#[tokio::test]
async fn the_preview_reads_the_current_publication_not_an_older_one() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;
    db.publish(MAIN, &[Line::manual("USD", "1", "33.00", "33.50")])
        .await;

    let answer = estimate(&db, &plan("USD", CustomerBuy, "10"))
        .await
        .expect("an estimate");

    assert_eq!(answer.thb, money("335.00"));
    db.finish().await;
}

#[tokio::test]
async fn the_preview_writes_no_quote_stock_or_booking_row() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD, JPY]).await;
    let before = db.untouchable_counts().await;
    let drafts = db.drafts().await;

    estimate(&db, &plan("USD", CustomerBuy, "100"))
        .await
        .expect("an estimate");
    estimate(&db, &plan("JPY", CustomerSell, "10000"))
        .await
        .expect("an estimate");
    let _ = estimate(&db, &plan("EUR", CustomerSell, "1")).await;

    assert_eq!(
        db.untouchable_counts().await,
        before,
        "P5: no quote, stock or booking row"
    );
    assert_eq!(
        db.drafts().await,
        drafts,
        "the preview saves no draft either"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_currency_with_no_published_line_is_rate_unavailable() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;

    let refused = estimate(&db, &plan("EUR", CustomerBuy, "100"))
        .await
        .expect_err("no EUR line");

    assert_eq!(refused.code(), "rateUnavailable");
    db.finish().await;
}

#[tokio::test]
async fn a_branch_with_no_publication_is_rate_unavailable() {
    let db = TestDb::start().await;

    let refused = estimate(&db, &plan("USD", CustomerBuy, "100"))
        .await
        .expect_err("nothing published");

    assert_eq!(refused.code(), "rateUnavailable");
    db.finish().await;
}

#[tokio::test]
async fn an_unavailable_side_is_rate_unavailable() {
    let db = TestDb::start().await;
    let mut usd_buy_only = USD;
    usd_buy_only.sell = None;
    db.publish(MAIN, &[usd_buy_only]).await;

    let refused = estimate(&db, &plan("USD", CustomerBuy, "100"))
        .await
        .expect_err("CUSTOMER_BUY needs the shop's sell side, which is NULL");
    let fine = estimate(&db, &plan("USD", CustomerSell, "100"))
        .await
        .expect("the buy side exists");

    assert_eq!(refused.code(), "rateUnavailable");
    assert_eq!(fine.thb, money("3210.00"));
    db.finish().await;
}

#[tokio::test]
async fn a_stale_line_under_stop_is_rate_unavailable() {
    let db = TestDb::start().await;
    let policy = db.policy(MAIN, Some("STOP"), None).await;
    db.publish(MAIN, &[USD.stale(&policy)]).await;

    let refused = estimate(&db, &plan("USD", CustomerBuy, "1"))
        .await
        .expect_err("STOP refuses");

    assert_eq!(refused.code(), "rateUnavailable");
    db.finish().await;
}

#[tokio::test]
async fn a_stale_line_under_limit_above_the_limit_is_rate_unavailable() {
    let db = TestDb::start().await;
    let policy = db.policy(MAIN, Some("LIMIT"), Some("1000")).await;
    db.publish(MAIN, &[USD.stale(&policy)]).await;

    let refused = estimate(&db, &plan("USD", CustomerBuy, "100"))
        .await
        .expect_err("3 245.00 THB is above the 1 000 limit");

    assert_eq!(refused.code(), "rateUnavailable");
    db.finish().await;
}

#[tokio::test]
async fn a_stale_line_under_limit_within_the_limit_is_estimated_and_marked_stale() {
    let db = TestDb::start().await;
    let policy = db.policy(MAIN, Some("LIMIT"), Some("1000")).await;
    db.publish(MAIN, &[USD.stale(&policy)]).await;

    let answer = estimate(&db, &plan("USD", CustomerBuy, "10"))
        .await
        .expect("324.50 ≤ 1 000");

    assert_eq!(answer.thb, money("324.50"));
    assert!(answer.stale);
    assert!(
        answer.source_date.is_some(),
        "a stale AUTO line carries its source date"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_stale_line_under_a_policy_with_no_rule_is_estimated_and_marked_stale() {
    let db = TestDb::start().await;
    let policy = db.policy(MAIN, None, None).await;
    db.publish(MAIN, &[USD.stale(&policy)]).await;

    let answer = estimate(&db, &plan("USD", CustomerSell, "100"))
        .await
        .expect("NULL admits");

    assert_eq!(answer.thb, money("3210.00"));
    assert!(answer.stale);
    db.finish().await;
}

#[tokio::test]
async fn a_plan_with_no_branch_is_validation_failed_on_branch() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;
    let mut no_branch = plan("USD", CustomerBuy, "100");
    no_branch.branch_id = None;

    let refused = estimate(&db, &no_branch)
        .await
        .expect_err("which branch's rates?");

    assert_eq!(
        refused,
        CustomerAppError::ValidationFailed { field: "branch" }
    );
    db.finish().await;
}
