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
//!
//! **Restated 2026-10-27 by the sub-controller claude/451f0116 (XC-V9-PRICING PV-h, #5168)** on the owner's ruling
//! B4 (`docs/design/blueprint/pricing-tiers.md` §15.6): `line_for` answers tier 1 of a tiered line whatever the
//! quantity and never reads the quantity or the base rates, so it has no arithmetic left to overflow. Rows 1–4 and 6
//! now read: rows 1–2 → `Ok(Some(T1))` even for `Decimal::MAX`; row 3 → T1 for every amount; row 4 → T1 without a
//! quantity; row 6 → a missing or disagreeing base rate and a gap change nothing (T1), and tier 1's side NULL →
//! `Ok(None)`. Row 7 is unchanged: an overflowing quantity is still refused, by `quote_in`'s own arithmetic.

mod quote_common;

use std::str::FromStr;

use pos_error::{AppError, AppResult};
use quote_common::*;
use rust_decimal::Decimal;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
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

fn assert_tier_1(res: AppResult<Option<PublishedSide>>, rate: &str, what: &str) {
    let side = res
        .unwrap_or_else(|e| panic!("{what}: {e:?}"))
        .unwrap_or_else(|| panic!("{what}: tier 1"));
    assert_eq!(side.value_tier.as_str(), "T1", "{what}");
    assert_eq!(side.rate.value(), dec(rate), "{what}");
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
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

/// Row 1, restated for B4: quantity `Decimal::MAX` (base rate `33.50`, unit `1`) → `Ok(Some(T1))`; the quantity is
/// not read, so nothing overflows and nothing panics.
#[tokio::test]
async fn row_1_decimal_max_quantity_answers_tier_1() {
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

    assert_tier_1(res, "33.00", "row 1");
    db.finish().await;
}

/// Row 2, restated for B4: quantity `Decimal::MAX` with base rate `1` and unit `0.0001` → `Ok(Some(T1))`.
#[tokio::test]
async fn row_2_decimal_max_quantity_on_a_small_unit_answers_tier_1() {
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

    assert_tier_1(res, "1.00", "row 2");
    db.finish().await;
}

/// Row 3, restated for B4: an ordinary quantity in any band → `Ok(Some(T1))`.
#[tokio::test]
async fn row_3_ordinary_quantity_in_any_band_returns_tier_1() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;

    // 100 × 33.50 = 3,350 THB (T1's band) and 500 × 33.50 = 16,750 THB (T2's band): both T1.
    for quantity in ["100", "500"] {
        assert_tier_1(
            try_line_for(&db, "USD", Side::Buy, "ALL", Some(qty(quantity))).await,
            "33.00",
            &format!("row 3, {quantity}"),
        );
    }

    db.finish().await;
}

/// Row 4, restated for B4: no quantity on a tiered line → `Ok(Some(T1))`.
#[tokio::test]
async fn row_4_no_quantity_on_tiered_line_returns_tier_1() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;

    assert_tier_1(
        try_line_for(&db, "USD", Side::Buy, "ALL", None).await,
        "33.00",
        "row 4",
    );

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

/// Row 6, restated for B4: a missing or disagreeing base rate (even with `Decimal::MAX`) and a value in a gap
/// change nothing (`Ok(Some(T1))`); tier 1's side NULL → `Ok(None)`, with no other tier used instead.
#[tokio::test]
async fn row_6_base_rates_and_gaps_are_not_read_and_a_null_tier_1_side_returns_none() {
    let db = TestDb::start().await;
    db.exec(
        "INSERT INTO branch_currencies (branch_id, currency_code, group_code, display) \
         VALUES ('MAIN', 'USD', 'MISSING', 'SHOW'), ('MAIN', 'USD', 'GAP', 'SHOW')",
    )
    .await;
    let p = db.publish("MAIN", &[USD]).await;

    // Group "MISSING": base_sell is missing on T2, base_buy disagrees between T1 and T2.
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

    assert_tier_1(
        try_line_for(
            &db,
            "USD",
            Side::Sell,
            "MISSING",
            Some(ForeignAmount::from_decimal(Decimal::MAX)),
        )
        .await,
        "34.20",
        "missing base SELL rate",
    );
    assert_tier_1(
        try_line_for(
            &db,
            "USD",
            Side::Buy,
            "MISSING",
            Some(ForeignAmount::from_decimal(Decimal::MAX)),
        )
        .await,
        "33.00",
        "disagreeing base BUY rate",
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

    assert_tier_1(
        try_line_for(&db, "USD", Side::Buy, "GAP", Some(qty("1000"))).await,
        "31.50",
        "a value in the gap",
    );
    for quantity in ["100", "2000"] {
        assert_eq!(
            try_line_for(&db, "USD", Side::Sell, "GAP", Some(qty(quantity)))
                .await
                .expect("ok"),
            None,
            "SELL {quantity}: tier 1's SELL is NULL, and T3 is never used instead"
        );
    }

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
        payment_method: None,
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
