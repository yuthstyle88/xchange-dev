//! APP-4 (#994) — `PlannerPreview::estimate` answers what the desk's `QuoteService::quote_in` answers
//! for the same published line: the same THB (JPY per 100 included), the same `stale` flag, and a
//! refusal (`rateUnavailable`) exactly where the quote refuses.
//! Written before the implementation by the ticket author's delegate (claude/dell-claude-3). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/customer-app.md` §4.3 (*"the preview reads the current publication
//! and answers `stale` and `rateUnavailable` like a quote"*), §9.1 row APP-4 (*"JPY 10 000 against
//! a JPY line of unit 100 gives the same THB as the desk quote for the same line"*, Blueprint §18
//! *"JPY/KRW per-unit กับ board-unit ได้ยอดเดียวกัน"*), P4 (`CUSTOMER_SELL` is the shop's `Buy`).
//! Ticket #994 §9.
//!
//! The harness is `quote_common` (#669's, used read-only). Each call runs in a transaction of its
//! own that is committed. Staleness is decided at read time on the wall clock (a reference fetched
//! 30 h ago against 24 h), so no case depends on timing.
//!
//! Amended by the ticket author (claude/89bd61d9, 2026-10-11) after main's #1100 made
//! `QuoteRequestLine.denomination_group` required. Lane controller's ruling on #994
//! (issuecomment-6101212412, option A): the preview prices each line like the kiosk
//! (`rate-feed-connector.md` §9.3), at the tier `line_for` picks for the quantity in the branch's
//! default group, which is `ALL` until groups exist. So the desk is asked for group `ALL`, and a
//! tiered `(USD, ALL)` must give the same THB, rate and tier in both.
//!
//! Signatures this test assumes: `xchange::application::workflows::exchange::quote::QuoteService`
//! as merged (#669); `xc_app::application::planner::planner_preview::PlannerPreview` and the
//! `xc_app` types of the scaffold commit before this one.

mod quote_common;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use quote_common::*;
use rust_decimal::Decimal;
use std::str::FromStr;
use xc_app::application::planner::planner_preview::{PlannerEstimate, PlannerPreview};
use xc_app::domain::app_error::CustomerAppError;
use xc_app::domain::planner::planner_draft::{DraftAmount, DraftPlan};
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

const BRANCH: &str = "MAIN";

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-7".to_owned(),
        username: "user-employee-7".to_owned(),
        employee_id: Some("employee-7".to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

fn amount(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(Decimal::from_str(text).expect("a decimal"))
}

async fn desk(db: &TestDb, code: &str, side: CustomerSide, quantity: &str) -> AppResult<Quote> {
    let service = QuoteService::new(db.pool.clone());
    let req = QuoteRequest {
        branch_id: BranchId::new(BRANCH),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines: vec![QuoteRequestLine {
            currency: CurrencyCode::parse(code).expect("a currency"),
            shop_side: side.to_shop(),
            quantity: amount(quantity),
            fee_thb: None,
            denomination_group: GroupCode::all(),
        }],
    };
    let mut tx = db.pool.begin().await.expect("begin");
    let answer = service
        .quote_in(&mut tx, req, &teller(), whole_second_now())
        .await;
    tx.commit().await.expect("commit");
    answer
}

async fn preview(
    db: &TestDb,
    code: &str,
    side: CustomerSide,
    quantity: &str,
) -> Result<PlannerEstimate, CustomerAppError> {
    let plan = DraftPlan {
        branch_id: Some(BranchId::new(BRANCH)),
        currency: CurrencyCode::parse(code).expect("a currency"),
        customer_side: side,
        amount: DraftAmount::Quantity(amount(quantity)),
    };
    let preview = PlannerPreview::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let answer = preview.estimate(&mut tx, &plan, now()).await;
    tx.commit().await.expect("commit");
    answer
}

fn now() -> DateTime<Utc> {
    whole_second_now()
}

/// Both answer, with the same THB and the same `stale`; or both refuse with `rateUnavailable`.
async fn assert_same(db: &TestDb, code: &str, side: CustomerSide, quantity: &str) {
    let quote = desk(db, code, side, quantity).await;
    let estimate = preview(db, code, side, quantity).await;
    match (quote, estimate) {
        (Ok(q), Ok(e)) => {
            assert_eq!(
                e.thb, q.lines[0].thb_amount,
                "{code} {side:?} {quantity}: THB"
            );
            assert_eq!(e.stale, q.stale, "{code} {side:?} {quantity}: stale");
            assert_eq!(
                e.rate, q.lines[0].final_rate,
                "{code} {side:?} {quantity}: rate"
            );
        }
        (Err(q), Err(e)) => {
            assert!(
                q.to_string().contains("rateUnavailable"),
                "the desk refused with {q}, not rateUnavailable"
            );
            assert_eq!(e.code(), "rateUnavailable", "{code} {side:?} {quantity}");
        }
        (q, e) => panic!(
            "{code} {side:?} {quantity}: the desk answered {:?} but the preview answered {:?}",
            q.map(|q| q.lines[0].thb_amount),
            e
        ),
    }
}

#[tokio::test]
async fn jpy_10000_against_a_jpy_line_of_unit_100_gives_the_desk_quotes_thb() {
    let db = TestDb::start().await;
    db.publish(BRANCH, &[JPY]).await;

    let quote = desk(&db, "JPY", CustomerSide::CustomerSell, "10000")
        .await
        .expect("the desk quotes");
    let estimate = preview(&db, "JPY", CustomerSide::CustomerSell, "10000")
        .await
        .expect("the preview estimates");

    assert_eq!(estimate.thb, quote.lines[0].thb_amount);
    assert_eq!(
        estimate.thb,
        xc_kernel::money::Money::parse("2350.00").expect("THB")
    );
    assert_same(&db, "JPY", CustomerSide::CustomerBuy, "10000").await;
    db.finish().await;
}

#[tokio::test]
async fn usd_both_sides_give_the_desk_quotes_thb() {
    let db = TestDb::start().await;
    db.publish(BRANCH, &[USD]).await;

    assert_same(&db, "USD", CustomerSide::CustomerBuy, "123.45").await;
    assert_same(&db, "USD", CustomerSide::CustomerSell, "500").await;
    db.finish().await;
}

#[tokio::test]
async fn a_missing_line_is_refused_by_both() {
    let db = TestDb::start().await;
    db.publish(BRANCH, &[USD]).await;

    assert_same(&db, "EUR", CustomerSide::CustomerBuy, "10").await;
    db.finish().await;
}

#[tokio::test]
async fn a_stale_line_is_refused_or_admitted_alike_under_each_policy() {
    let db = TestDb::start().await;
    let stop = db.policy(BRANCH, Some("STOP"), None).await;
    db.publish(
        BRANCH,
        &[Line {
            stale_under: Some(stop),
            ..USD
        }],
    )
    .await;
    assert_same(&db, "USD", CustomerSide::CustomerBuy, "1").await;

    let limit = db.policy(BRANCH, Some("LIMIT"), Some("1000")).await;
    db.publish(
        BRANCH,
        &[Line {
            stale_under: Some(limit),
            ..USD
        }],
    )
    .await;
    assert_same(&db, "USD", CustomerSide::CustomerBuy, "10").await;
    assert_same(&db, "USD", CustomerSide::CustomerBuy, "100").await;

    let open = db.policy(BRANCH, None, None).await;
    db.publish(
        BRANCH,
        &[Line {
            stale_under: Some(open),
            ..USD
        }],
    )
    .await;
    assert_same(&db, "USD", CustomerSide::CustomerSell, "100").await;
    db.finish().await;
}

/// `(tier, lower exclusive THB, upper inclusive THB, base buy, base sell, buy, sell)`, as in
/// `rate_feed_tiers_test.rs` (#1100).
type Tier = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

/// Three bands of `(USD, ALL)`: ≤ 10,000 THB, ≤ 50,000 THB, above; base 32.00 / 32.50.
const USD_TIERS: [Tier; 3] = [
    (
        "T1",
        None,
        Some("10000.00"),
        Some("32.00"),
        Some("32.50"),
        Some("31.50"),
        Some("33.00"),
    ),
    (
        "T2",
        Some("10000.00"),
        Some("50000.00"),
        Some("32.00"),
        Some("32.50"),
        Some("31.70"),
        Some("32.80"),
    ),
    (
        "T3",
        Some("50000.00"),
        None,
        Some("32.00"),
        Some("32.50"),
        Some("31.90"),
        Some("32.60"),
    ),
];

fn sql_opt(v: Option<&str>) -> String {
    v.map_or("NULL".to_owned(), |s| s.to_owned())
}

/// Replaces the untiered `(USD, ALL)` line of `publication` with the three tier lines.
async fn tier_usd(db: &TestDb, publication: &str) {
    for (tier, lo, hi, base_buy, base_sell, buy, sell) in USD_TIERS {
        db.exec(&format!(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                 denomination_group, unit_size, buy_rate, sell_rate, source_snapshot, value_tier, \
                 tier_lower_exclusive_thb, tier_upper_inclusive_thb, base_buy_rate, base_sell_rate) \
             SELECT '{id}', organization_id, publication_id, currency_code, 'ALL', 1, \
                 {buy}, {sell}, source_snapshot, '{tier}', {lo}, {hi}, {base_buy}, {base_sell} \
             FROM published_rate_lines \
             WHERE publication_id = '{publication}' AND currency_code = 'USD' \
               AND denomination_group = 'ALL' AND value_tier = 'ALL'",
            id = uuid::Uuid::new_v4(),
            buy = sql_opt(buy),
            sell = sql_opt(sell),
            lo = sql_opt(lo),
            hi = sql_opt(hi),
            base_buy = sql_opt(base_buy),
            base_sell = sql_opt(base_sell),
        ))
        .await;
    }
    db.exec(&format!(
        "DELETE FROM published_rate_lines WHERE publication_id = '{publication}' \
           AND currency_code = 'USD' AND denomination_group = 'ALL' AND value_tier = 'ALL'"
    ))
    .await;
}

#[tokio::test]
async fn a_tiered_line_is_priced_at_the_tier_of_the_quantity_like_the_desk() {
    let db = TestDb::start().await;
    let p = db.publish(BRANCH, &[USD]).await;
    tier_usd(&db, &p.id).await;

    // CUSTOMER_SELL is the shop's BUY (P4): base 32.00. 100 → 3,200 (T1); 312.50 → 10,000 exactly
    // (T1, the upper edge is inclusive); 312.51 → T2; 2000 → 64,000 (T3).
    for quantity in ["100", "312.50", "312.51", "2000"] {
        assert_same(&db, "USD", CustomerSide::CustomerSell, quantity).await;
    }
    // CUSTOMER_BUY is the shop's SELL: base 32.50. 308 → 10,010 (T2, where base 32.00 would give T1).
    assert_same(&db, "USD", CustomerSide::CustomerBuy, "308").await;

    let t2 = preview(&db, "USD", CustomerSide::CustomerSell, "312.51")
        .await
        .expect("the preview estimates a T2 amount");
    assert_eq!(
        t2.rate,
        xc_kernel::money::Rate::parse("31.70").expect("a rate"),
        "the T2 rate, not the first tier's 31.50"
    );
    db.finish().await;
}
