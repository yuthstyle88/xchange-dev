//! Ticket #2788 (child of #2063, K-a) — `PgPublishedRates::line_for` tiered branch (`published_rates.rs:247`)
//! uses `ExchangeArithmetic::try_thb` instead of the unchecked `ExchangeArithmetic::thb`.
//!
//! Behaviour table (§8) rows 1–7:
//! 1. `quantity × base_rate` overflows `Decimal` (quantity `Decimal::MAX`, base rate `33.50`, unit `1`)
//!    → `Err(AppError::Domain(s))`, `s` starts with `validationFailed: quantity`; no panic.
//! 2. product fits but `÷ unit_size` overflows (quantity `Decimal::MAX`, base rate `1`, unit `0.0001`)
//!    → same as row 1.
//! 3. kept: ordinary quantity in a band → `Ok(Some(side))` with the tier of that band.
//! 4. kept: no quantity on a tiered line → `Ok(None)`.
//! 5. kept: untiered (`'ALL'`) line with any quantity, including `Decimal::MAX` → `Ok(Some(..))`.
//! 6. kept: missing/disagreeing base rate (even with `Decimal::MAX`), no band containing the value,
//!    picked side NULL → `Ok(None)`.
//! 7. via `QuoteService::quote_in` on a tiered line with an overflowing quantity
//!    → `Err(AppError::Domain(s))`, `s` starts with `validationFailed: quantity` (was: panic).

mod quote_common;

use std::str::FromStr;

use pos_error::{AppError, AppResult};
use quote_common::*;
use rust_decimal::Decimal;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates, PublishedSide};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

/// One tier line: `(tier, lower_exclusive, upper_inclusive, base_buy, base_sell, buy, sell)`.
type Tier = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

const USD_TIERS: [Tier; 3] = [
    (
        "T1",
        None,
        Some("10000.00"),
        Some("33.50"),
        Some("34.00"),
        Some("33.00"),
        Some("34.20"),
    ),
    (
        "T2",
        Some("10000.00"),
        Some("50000.00"),
        Some("33.50"),
        Some("34.00"),
        Some("33.20"),
        Some("34.10"),
    ),
    (
        "T3",
        Some("50000.00"),
        None,
        Some("33.50"),
        Some("34.00"),
        Some("33.40"),
        Some("34.05"),
    ),
];

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known currency")
}

fn qty(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(dec(text))
}

fn group(text: &str) -> GroupCode {
    GroupCode::new(text).expect("a valid group code")
}

fn sql_opt(v: Option<&str>) -> String {
    v.map_or("NULL".to_owned(), |s| s.to_owned())
}

async fn add_tiers(
    db: &TestDb,
    publication: &str,
    currency: &str,
    grp: &str,
    unit_size: &str,
    tiers: &[Tier],
    drop_all: bool,
) -> Vec<(String, String)> {
    let mut ids = Vec::new();
    for (tier, lo, hi, base_buy, base_sell, buy, sell) in tiers {
        let id = uuid::Uuid::new_v4().to_string();
        db.exec(&format!(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                 denomination_group, unit_size, buy_rate, sell_rate, source_snapshot, value_tier, \
                 tier_lower_exclusive_thb, tier_upper_inclusive_thb, base_buy_rate, base_sell_rate) \
             SELECT '{id}', organization_id, publication_id, currency_code, '{grp}', {unit_size}, \
                 {buy}, {sell}, source_snapshot, '{tier}', {lo}, {hi}, {base_buy}, {base_sell} \
             FROM published_rate_lines \
             WHERE publication_id = '{publication}' AND currency_code = '{currency}' \
               AND denomination_group = 'ALL' AND value_tier = 'ALL'",
            buy = sql_opt(*buy),
            sell = sql_opt(*sell),
            lo = sql_opt(*lo),
            hi = sql_opt(*hi),
            base_buy = sql_opt(*base_buy),
            base_sell = sql_opt(*base_sell),
        ))
        .await;
        ids.push((tier.to_string(), id));
    }
    if drop_all {
        db.exec(&format!(
            "DELETE FROM published_rate_lines WHERE publication_id = '{publication}' \
               AND currency_code = '{currency}' AND denomination_group = 'ALL' AND value_tier = 'ALL'"
        ))
        .await;
    }
    ids
}

async fn try_line_for(
    db: &TestDb,
    currency: &str,
    side: Side,
    grp: &str,
    quantity: Option<ForeignAmount>,
) -> AppResult<Option<PublishedSide>> {
    let rates = PgPublishedRates::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let res = rates
        .line_for(
            &mut tx,
            &BranchId::new("MAIN"),
            LineSelector {
                code: code(currency),
                side,
                group: group(grp),
                quantity,
            },
        )
        .await;
    tx.commit().await.expect("commit");
    res
}

fn assert_quantity_overflow<T: std::fmt::Debug>(res: AppResult<T>, what: &str) {
    match res {
        Err(AppError::Domain(msg)) => {
            assert!(
                msg.starts_with("validationFailed: quantity"),
                "{what}: expected domain error starting with `validationFailed: quantity`, got {msg:?}"
            );
            assert_eq!(
                msg, "validationFailed: quantity overflows thb amount",
                "{what}: exact refusal text"
            );
        }
        other => panic!(
            "{what}: expected Err(AppError::Domain(\"validationFailed: quantity ...\")), got {other:?}"
        ),
    }
}

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-7".to_owned(),
        username: "user-employee-7".to_owned(),
        employee_id: Some("employee-7".to_owned()),
        role: Role::Front,
    }
}

/// Row 1: `quantity × base_rate` overflows `Decimal` (quantity `Decimal::MAX`, base rate `33.50`, unit `1`)
/// → `Err(AppError::Domain(s))`, `s` starts with `validationFailed: quantity`; no panic.
#[tokio::test]
async fn row_1_product_overflow_refused_as_validation_failed_quantity() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;

    let res = try_line_for(
        &db,
        "USD",
        Side::Buy,
        "ALL",
        Some(ForeignAmount::from_decimal(Decimal::MAX)),
    )
    .await;

    assert_quantity_overflow(res, "row 1 product overflow");
    db.finish().await;
}

/// Row 2: product fits but `÷ unit_size` overflows (quantity `Decimal::MAX`, base rate `1`, unit `0.0001`)
/// → `Err(AppError::Domain(s))`, `s` starts with `validationFailed: quantity`; no panic.
#[tokio::test]
async fn row_2_division_by_unit_size_overflow_refused_as_validation_failed_quantity() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let div_tiers: [Tier; 2] = [
        (
            "T1",
            None,
            Some("10000.00"),
            Some("1.00"),
            Some("1.00"),
            Some("1.00"),
            Some("1.00"),
        ),
        (
            "T2",
            Some("10000.00"),
            None,
            Some("1.00"),
            Some("1.00"),
            Some("1.00"),
            Some("1.00"),
        ),
    ];
    add_tiers(&db, &p.id, "USD", "ALL", "0.0001", &div_tiers, true).await;

    let res = try_line_for(
        &db,
        "USD",
        Side::Buy,
        "ALL",
        Some(ForeignAmount::from_decimal(Decimal::MAX)),
    )
    .await;

    assert_quantity_overflow(res, "row 2 division overflow");
    db.finish().await;
}

/// Row 3: kept: ordinary quantity in a band → `Ok(Some(side))` with the tier of that band.
#[tokio::test]
async fn row_3_ordinary_quantity_in_band_returns_matching_tier() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;

    // 100 * 33.50 = 3,350 THB -> T1
    let side_t1 = try_line_for(&db, "USD", Side::Buy, "ALL", Some(qty("100")))
        .await
        .expect("ok")
        .expect("tier found");
    assert_eq!(side_t1.value_tier.as_str(), "T1");
    assert_eq!(side_t1.rate.value(), dec("33.00"));

    // 500 * 33.50 = 16,750 THB -> T2
    let side_t2 = try_line_for(&db, "USD", Side::Buy, "ALL", Some(qty("500")))
        .await
        .expect("ok")
        .expect("tier found");
    assert_eq!(side_t2.value_tier.as_str(), "T2");
    assert_eq!(side_t2.rate.value(), dec("33.20"));

    db.finish().await;
}

/// Row 4: kept: no quantity on a tiered line → `Ok(None)`.
#[tokio::test]
async fn row_4_no_quantity_on_tiered_line_returns_none() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;

    let got = try_line_for(&db, "USD", Side::Buy, "ALL", None)
        .await
        .expect("ok");
    assert_eq!(got, None);

    db.finish().await;
}

/// Row 5: kept: untiered (`'ALL'`) line with any quantity, including `Decimal::MAX` → `Ok(Some(..))`.
#[tokio::test]
async fn row_5_untiered_line_with_decimal_max_returns_some() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;

    let side = try_line_for(
        &db,
        "USD",
        Side::Buy,
        "ALL",
        Some(ForeignAmount::from_decimal(Decimal::MAX)),
    )
    .await
    .expect("ok")
    .expect("untiered line never prices so never refuses");

    assert_eq!(side.line_id.to_string(), p.line("USD"));
    assert_eq!(side.value_tier.as_str(), "ALL");

    db.finish().await;
}

/// Row 6: kept: missing/disagreeing base rate (even with `Decimal::MAX`), no band containing the value,
/// picked side NULL → `Ok(None)`.
#[tokio::test]
async fn row_6_missing_or_disagreeing_base_rate_gap_or_null_side_returns_none() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;

    // Group "MISSING_BASE": base_sell is missing on T2, base_buy disagrees between T1 and T2.
    // Even with Decimal::MAX, base-rate check at :216-235 runs before :247 and returns Ok(None).
    let bad_base_tiers: [Tier; 2] = [
        (
            "T1",
            None,
            Some("10000.00"),
            Some("33.50"),
            Some("34.00"),
            Some("33.00"),
            Some("34.20"),
        ),
        (
            "T2",
            Some("10000.00"),
            None,
            Some("32.00"),
            None,
            Some("33.20"),
            Some("34.10"),
        ),
    ];
    add_tiers(&db, &p.id, "USD", "MISSING", "1", &bad_base_tiers, false).await;

    assert_eq!(
        try_line_for(
            &db,
            "USD",
            Side::Sell,
            "MISSING",
            Some(ForeignAmount::from_decimal(Decimal::MAX)),
        )
        .await
        .expect("ok"),
        None,
        "missing base rate with Decimal::MAX returns Ok(None) before arithmetic"
    );
    assert_eq!(
        try_line_for(
            &db,
            "USD",
            Side::Buy,
            "MISSING",
            Some(ForeignAmount::from_decimal(Decimal::MAX)),
        )
        .await
        .expect("ok"),
        None,
        "disagreeing base rate with Decimal::MAX returns Ok(None) before arithmetic"
    );

    // Group "GAP": gap between 10,000 and 50,000, and T1 SELL is NULL.
    let gap_tiers: [Tier; 2] = [
        (
            "T1",
            None,
            Some("10000.00"),
            Some("32.00"),
            Some("32.50"),
            Some("31.50"),
            None,
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
    add_tiers(&db, &p.id, "USD", "GAP", "1", &gap_tiers, false).await;

    assert_eq!(
        try_line_for(&db, "USD", Side::Buy, "GAP", Some(qty("1000")))
            .await
            .expect("ok"),
        None,
        "value in gap returns Ok(None)"
    );
    assert_eq!(
        try_line_for(&db, "USD", Side::Sell, "GAP", Some(qty("100")))
            .await
            .expect("ok"),
        None,
        "picked tier with NULL side returns Ok(None)"
    );

    db.finish().await;
}

/// Row 7: via `QuoteService::quote_in` on a tiered line with an overflowing quantity
/// → `Err(AppError::Domain(s))`, `s` starts with `validationFailed: quantity` (was: panic).
#[tokio::test]
async fn row_7_quote_in_on_tiered_line_with_overflowing_quantity_refused() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;
    let before = db.quote_rows().await;

    let service = QuoteService::new(db.pool.clone());
    let req = QuoteRequest {
        branch_id: BranchId::new("MAIN"),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines: vec![QuoteRequestLine {
            currency: code("USD"),
            shop_side: Side::Buy,
            quantity: ForeignAmount::from_decimal(Decimal::MAX),
            fee_thb: None,
            denomination_group: group("ALL"),
        }],
    };
    let mut tx = db.pool.begin().await.expect("begin");
    let res: AppResult<Quote> = service
        .quote_in(&mut tx, req, &teller(), whole_second_now())
        .await;
    tx.commit().await.expect("commit");

    assert_quantity_overflow(res, "row 7 quote_in tiered overflow");
    assert_eq!(db.quote_rows().await, before, "no quote rows written");
    db.finish().await;
}
