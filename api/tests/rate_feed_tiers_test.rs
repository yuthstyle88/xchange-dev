//! Rate feed F6 — reading a tiered price at the counter (`docs/design/rate-feed-connector.md`
//! §2, §3.4, §3.8, §4 item 3; owner's ruling R-4 on #986: every tier is published in advance and a
//! quote picks the tier from the transaction's value).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `quote_common/mod.rs`).
//!
//! **Restated 2026-10-27 by the sub-controller claude/451f0116 (XC-V9-PRICING PV-h, #5168)** on the owner's ruling
//! B4 (`docs/design/blueprint/pricing-tiers.md` §15.6; `H:` §8: the server quote takes the currency's buy/sell and
//! does not change the price with the amount). `line_for` answers **tier 1** of a tiered `(code, group)` (the line
//! with `tier_lower_exclusive_thb IS NULL`) whatever the quantity, with or without one; the base rates are not read.
//! Tier 1's side not priced, or no tier 1 line → `None`; another tier is never used instead. The untiered rows, the
//! `line_in` guard and the group rules below are unchanged. Where this header and the list below disagree, this
//! header wins.
//!
//! What this pins:
//! - `xc_rate::domain::publication::published_rates::LineSelector { code, side, group: GroupCode,
//!   quantity: Option<ForeignAmount> }` and `PublishedRates::line_for(tx, branch, LineSelector)
//!   -> AppResult<Option<PublishedSide>>`; `PublishedSide.value_tier: TierCode`.
//! - An untiered `(code, group)` (its one line has `value_tier = 'ALL'`) answers that line, with or
//!   without a quantity.
//! - A tiered `(code, group)`: the tier is picked on the **base** rate of the side,
//!   `ExchangeArithmetic::thb(quantity, base_rate, unit_size)`, with `lower < v <= upper`. The base
//!   rate of the side must be present and the same on every tier line of `(code, group)`. No
//!   quantity, no band containing the value, a missing or disagreeing base rate, or a picked tier
//!   whose side is not priced → `None`.
//! - `line_in` never reads a tiered line (the §4 item 3 guard): `None` on a tiered `(code, 'ALL')`.
//! - `QuoteRequestLine.denomination_group: GroupCode`: `quote_in` prices the asked group at its
//!   tier, and stores that group and that line.

mod quote_common;

use std::str::FromStr;

use chrono::{DateTime, Utc};
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

/// Three bands, BP §02 style: ≤ 10,000 THB, ≤ 50,000 THB, above.
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

/// Adds tier lines of `(currency, group)` with `unit_size` to publication `publication`, copying
/// the snapshot of that publication's `(currency, 'ALL')` line. With `drop_all`, the untiered
/// `(currency, 'ALL')` line is removed afterwards, so that `(currency, 'ALL')` is tiered.
/// Answers `(tier, line id)`.
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

fn id_of<'a>(ids: &'a [(String, String)], tier: &str) -> &'a str {
    ids.iter()
        .find(|(t, _)| t == tier)
        .map(|(_, id)| id.as_str())
        .unwrap_or_else(|| panic!("tier {tier} was added"))
}

async fn line_for(
    db: &TestDb,
    currency: &str,
    side: Side,
    grp: &str,
    quantity: Option<&str>,
) -> Option<PublishedSide> {
    let rates = PgPublishedRates::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let got = rates
        .line_for(
            &mut tx,
            &BranchId::new("MAIN"),
            LineSelector {
                code: code(currency),
                side,
                group: group(grp),
                quantity: quantity.map(qty),
            },
        )
        .await
        .expect("line_for answers");
    tx.commit().await.expect("commit");
    got
}

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-7".to_owned(),
        username: "user-employee-7".to_owned(),
        employee_id: Some("employee-7".to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

async fn quote(
    db: &TestDb,
    currency: &str,
    side: Side,
    grp: &str,
    quantity: &str,
    now: DateTime<Utc>,
) -> pos_error::AppResult<Quote> {
    let service = QuoteService::new(db.pool.clone());
    let req = QuoteRequest {
        branch_id: BranchId::new("MAIN"),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines: vec![QuoteRequestLine {
            currency: code(currency),
            shop_side: side,
            quantity: qty(quantity),
            fee_thb: None,
            denomination_group: group(grp),
        }],
        payment_method: None,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    let answer = service.quote_in(&mut tx, req, &teller(), now).await;
    tx.commit().await.expect("commit");
    answer
}

/// `(published_rate_line_id, denomination_group, final_rate, thb_amount)` of the quote's line.
async fn stored(db: &TestDb, q: &Quote) -> (String, String, Decimal, Decimal) {
    sqlx::query_as(
        "SELECT published_rate_line_id::text, denomination_group, final_rate, thb_amount \
         FROM quote_lines WHERE quote_id = $1::uuid",
    )
    .bind(q.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("the quote line is stored")
}

#[tokio::test]
async fn an_untiered_line_is_read_with_or_without_a_quantity() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;

    let side = line_for(&db, "USD", Side::Buy, "ALL", None)
        .await
        .expect("the ALL line");
    assert_eq!(side.line_id.to_string(), p.line("USD"));
    assert_eq!(side.rate.value(), dec("32.10"));
    assert_eq!(side.value_tier.as_str(), "ALL");

    let with_qty = line_for(&db, "USD", Side::Sell, "ALL", Some("100000"))
        .await
        .expect("a quantity changes nothing on an untiered line");
    assert_eq!(with_qty.line_id.to_string(), p.line("USD"));
    assert_eq!(with_qty.rate.value(), dec("32.45"));

    assert_eq!(
        line_for(&db, "USD", Side::Buy, "LARGE", Some("100")).await,
        None,
        "a group that is not published"
    );
    db.finish().await;
}

#[tokio::test]
async fn every_quantity_is_priced_at_tier_1() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let ids = add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;

    // B4: 3,200.00 · 10,000.00 · 10,000.32 · 50,000.00 · 64,000.00 THB at base 32.00 used to pick T1, T1, T2, T2, T3.
    for quantity in ["100", "312.50", "312.51", "1562.50", "2000"] {
        let side = line_for(&db, "USD", Side::Buy, "ALL", Some(quantity))
            .await
            .unwrap_or_else(|| panic!("tier 1 for {quantity}"));
        assert_eq!(side.value_tier.as_str(), "T1", "BUY {quantity}");
        assert_eq!(
            side.line_id.to_string(),
            id_of(&ids, "T1"),
            "BUY {quantity}"
        );
        assert_eq!(side.rate.value(), dec("31.50"), "BUY {quantity}");
    }

    let sell = line_for(&db, "USD", Side::Sell, "ALL", Some("308"))
        .await
        .expect("tier 1's SELL");
    assert_eq!(sell.value_tier.as_str(), "T1", "B4: no SELL tier by amount");
    assert_eq!(sell.rate.value(), dec("33.00"));
    db.finish().await;
}

#[tokio::test]
async fn a_tiered_line_without_a_quantity_is_tier_1() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let ids = add_tiers(&db, &p.id, "USD", "ALL", "1", &USD_TIERS, true).await;
    let side = line_for(&db, "USD", Side::Buy, "ALL", None)
        .await
        .expect("B4: tier 1 needs no quantity");
    assert_eq!(side.line_id.to_string(), id_of(&ids, "T1"));
    assert_eq!(side.rate.value(), dec("31.50"));
    db.finish().await;
}

#[tokio::test]
async fn tier_1_whose_side_is_not_priced_is_unavailable_and_no_other_tier_is_used() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    // T1 ≤ 10,000; T3 > 50,000; nothing in between. T1 has no SELL price (hidden or paused).
    let tiers: [Tier; 2] = [
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
    add_tiers(&db, &p.id, "USD", "ALL", "1", &tiers, true).await;

    let buy = line_for(&db, "USD", Side::Buy, "ALL", Some("1000"))
        .await
        .expect("B4: 32,000.00 THB, once in the gap, is tier 1");
    assert_eq!(buy.value_tier.as_str(), "T1");
    assert_eq!(buy.rate.value(), dec("31.50"));
    for quantity in ["100", "2000"] {
        assert_eq!(
            line_for(&db, "USD", Side::Sell, "ALL", Some(quantity)).await,
            None,
            "SELL {quantity}: tier 1's SELL is not priced, and T3 is never used instead"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn the_base_rates_are_not_read() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    // The base SELL is missing on T2 and the base BUY differs between the two lines: B4 reads neither.
    let tiers: [Tier; 2] = [
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
            None,
            Some("31.00"),
            None,
            Some("31.70"),
            Some("32.80"),
        ),
    ];
    add_tiers(&db, &p.id, "USD", "ALL", "1", &tiers, true).await;

    let sell = line_for(&db, "USD", Side::Sell, "ALL", Some("100"))
        .await
        .expect("tier 1's SELL");
    assert_eq!(sell.rate.value(), dec("33.00"));
    let buy = line_for(&db, "USD", Side::Buy, "ALL", Some("100"))
        .await
        .expect("tier 1's BUY");
    assert_eq!(buy.rate.value(), dec("31.50"));
    db.finish().await;
}

#[tokio::test]
async fn jpy_is_tiered_per_its_own_unit_size() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[JPY]).await;
    // Board 0.2301 per 1, published per 100 (design §3.8): base BUY 22.00 per 100.
    let tiers: [Tier; 2] = [
        (
            "T1",
            None,
            Some("10000.00"),
            Some("22.00"),
            Some("23.00"),
            Some("23.01"),
            Some("24.00"),
        ),
        (
            "T2",
            Some("10000.00"),
            None,
            Some("22.00"),
            Some("23.00"),
            Some("23.10"),
            Some("23.90"),
        ),
    ];
    add_tiers(&db, &p.id, "JPY", "ALL", "100", &tiers, true).await;

    // 10,000 JPY × 22.00 ÷ 100 = 2,200.00 THB → T1 (it would be T2 if the unit were ignored).
    let side = line_for(&db, "JPY", Side::Buy, "ALL", Some("10000"))
        .await
        .expect("a JPY tier");
    assert_eq!(side.value_tier.as_str(), "T1");
    assert_eq!(side.rate.value(), dec("23.01"));
    assert_eq!(side.unit_size.value(), dec("100"));
    db.finish().await;
}

#[tokio::test]
async fn each_group_has_its_own_lines() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    // LARGE is tiered; ALL stays the untiered line.
    let ids = add_tiers(&db, &p.id, "USD", "LARGE", "1", &USD_TIERS, false).await;

    let large = line_for(&db, "USD", Side::Buy, "LARGE", Some("2000"))
        .await
        .expect("LARGE's tier 1");
    assert_eq!(large.line_id.to_string(), id_of(&ids, "T1"), "B4");
    let all = line_for(&db, "USD", Side::Buy, "ALL", Some("2000"))
        .await
        .expect("the ALL line");
    assert_eq!(all.line_id.to_string(), p.line("USD"));
    assert_eq!(all.value_tier.as_str(), "ALL");
    db.finish().await;
}

#[tokio::test]
async fn a_quote_prices_the_asked_group_at_tier_1_and_stores_both() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let ids = add_tiers(&db, &p.id, "USD", "LARGE", "1", &USD_TIERS, false).await;
    let now = whole_second_now();

    // B4: 500 × base 32.00 = 16,000.00 THB used to pick T2; it is priced at T1's 31.50 → 15,750.00 THB.
    let q = quote(&db, "USD", Side::Buy, "LARGE", "500", now)
        .await
        .unwrap_or_else(|e| panic!("quote_in refused: {e}"));
    let (line_id, grp, final_rate, thb) = stored(&db, &q).await;
    assert_eq!(line_id, id_of(&ids, "T1"));
    assert_eq!(grp, "LARGE");
    assert_eq!(final_rate, dec("31.50"));
    assert_eq!(thb, dec("15750.00"));

    // The ALL group is still the untiered line.
    let q = quote(&db, "USD", Side::Buy, "ALL", "500", now)
        .await
        .unwrap_or_else(|e| panic!("quote_in refused: {e}"));
    let (line_id, grp, final_rate, _) = stored(&db, &q).await;
    assert_eq!(line_id, p.line("USD"));
    assert_eq!(grp, "ALL");
    assert_eq!(final_rate, dec("32.10"));
    db.finish().await;
}

#[tokio::test]
async fn a_quote_on_tiers_with_no_tier_1_is_refused_and_nothing_is_stored() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    // No line is open below (`tier_lower_exclusive_thb IS NULL`): there is no tier 1 to price at.
    let tiers: [Tier; 1] = [(
        "T2",
        Some("10000.00"),
        None,
        Some("32.00"),
        Some("32.50"),
        Some("31.70"),
        Some("32.80"),
    )];
    add_tiers(&db, &p.id, "USD", "ALL", "1", &tiers, true).await;
    let before = db.quote_rows().await;

    let answer = quote(&db, "USD", Side::Buy, "ALL", "1000", whole_second_now()).await;
    let err = answer.expect_err("no tier 1");
    assert!(err.to_string().contains("rateUnavailable"), "{err}");
    assert_eq!(db.quote_rows().await, before, "nothing stored");
    db.finish().await;
}
