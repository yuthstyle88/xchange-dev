//! Rate feed F8 — bookings priced at the tier of the booked amount
//! (`docs/design/rate-feed-connector.md` §3.4, §6 F8; owner's ruling R-4 on #986).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `quote_common/mod.rs`).
//!
//! What this pins:
//! - `PgBookingPorts::counter_rate_in(tx, branch, code, side, amount)` reads the counter line with
//!   `PublishedRates::line_for` (group `ALL`, quantity = `amount` as a foreign amount): on a tiered
//!   publication it answers the tier of the amount; with no amount, a tiered line is unavailable;
//!   an untiered line answers as before, with or without an amount.
//! - Confirming a pending member booking (`BookingService::confirm`) reads the counter at the
//!   booking's own amount, so a booking created on a tiered publication can be confirmed while that
//!   publication is current.

mod quote_common;

use std::str::FromStr;
use std::sync::Arc;

use chrono::{Duration, Utc};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use xc_exchange::application::booking::service::BookingService;
use xc_exchange::domain::booking::commands::confirm_booking::ConfirmBooking;
use xc_exchange::domain::booking::repositories::booking_ports::{BookingPorts, CounterLine};
use xc_exchange::infrastructure::booking::booking_ids::UuidBookingIds;
use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;
use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Side};
use xchange::infrastructure::bookings::booking_ports::PgBookingPorts;

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

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("USD")
}

fn sql_opt(v: Option<&str>) -> String {
    v.map_or("NULL".to_owned(), |s| s.to_owned())
}

/// Replaces the `(USD, 'ALL')` line of `publication` with the three tiers above. Answers
/// `(tier, line id)`.
async fn tier_usd(db: &TestDb, publication: &str) -> Vec<(String, String)> {
    let mut ids = Vec::new();
    for (tier, lo, hi, base_buy, base_sell, buy, sell) in USD_TIERS {
        let id = uuid::Uuid::new_v4().to_string();
        db.exec(&format!(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                 denomination_group, unit_size, buy_rate, sell_rate, source_snapshot, value_tier, \
                 tier_lower_exclusive_thb, tier_upper_inclusive_thb, base_buy_rate, base_sell_rate) \
             SELECT '{id}', organization_id, publication_id, currency_code, 'ALL', 1, {buy}, {sell}, \
                 source_snapshot, '{tier}', {lo}, {hi}, {base_buy}, {base_sell} \
             FROM published_rate_lines WHERE publication_id = '{publication}' \
               AND currency_code = 'USD' AND value_tier = 'ALL'",
            buy = sql_opt(buy),
            sell = sql_opt(sell),
            lo = sql_opt(lo),
            hi = sql_opt(hi),
            base_buy = sql_opt(base_buy),
            base_sell = sql_opt(base_sell),
        ))
        .await;
        ids.push((tier.to_string(), id));
    }
    db.exec(&format!(
        "DELETE FROM published_rate_lines WHERE publication_id = '{publication}' \
           AND currency_code = 'USD' AND value_tier = 'ALL'"
    ))
    .await;
    ids
}

fn id_of<'a>(ids: &'a [(String, String)], tier: &str) -> &'a str {
    ids.iter()
        .find(|(t, _)| t == tier)
        .map(|(_, id)| id.as_str())
        .unwrap_or_else(|| panic!("tier {tier}"))
}

async fn counter(db: &TestDb, side: Side, amount: Option<&str>) -> Option<CounterLine> {
    let ports = PgBookingPorts::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let got = ports
        .counter_rate_in(
            &mut tx,
            &BranchId::new("MAIN"),
            &usd(),
            side,
            amount.map(|a| Money::parse(a).expect("an amount")),
        )
        .await
        .expect("counter_rate_in answers");
    tx.commit().await.expect("commit");
    got
}

#[tokio::test]
async fn a_tiered_counter_rate_is_the_tier_of_the_booked_amount() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let ids = tier_usd(&db, &p.id).await;

    // 500 USD × base 32.00 = 16,000.00 THB → T2.
    let line = counter(&db, Side::Buy, Some("500.00"))
        .await
        .expect("a T2 line");
    assert_eq!(line.line_id.to_string(), id_of(&ids, "T2"));
    assert_eq!(line.rate.value(), dec("31.70"));

    // 100 USD × base 32.50 = 3,250.00 THB → T1 (SELL picks on the base SELL).
    let line = counter(&db, Side::Sell, Some("100.00"))
        .await
        .expect("a T1 line");
    assert_eq!(line.line_id.to_string(), id_of(&ids, "T1"));
    assert_eq!(line.rate.value(), dec("33.00"));
    db.finish().await;
}

#[tokio::test]
async fn a_tiered_counter_rate_without_an_amount_is_unavailable() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    tier_usd(&db, &p.id).await;
    assert!(counter(&db, Side::Buy, None).await.is_none());
    db.finish().await;
}

#[tokio::test]
async fn an_untiered_counter_rate_is_read_as_before() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    for amount in [None, Some("500.00"), Some("100000.00")] {
        let line = counter(&db, Side::Buy, amount).await.expect("the ALL line");
        assert_eq!(line.line_id.to_string(), p.line("USD"), "{amount:?}");
        assert_eq!(line.rate.value(), dec("32.10"), "{amount:?}");
    }
    db.finish().await;
}

/// A pending member booking `BK-TIER` of 500 USD bought at 31.70, quoted from `line_id`.
async fn pending_booking(db: &TestDb, line_id: &str) {
    db.customer("CU-TIER", "MAIN").await;
    db.exec(
        "INSERT INTO members (id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by) \
         VALUES ('MB-TIER', 'MAIN', 'CU-TIER', 'Tier Member', '0800000001', 'tier-token', now(), 'owner')",
    )
    .await;
    // A pending member booking of 500 USD bought at T2 (31.70), quoted from the T2 line.
    db.exec(&format!(
        "INSERT INTO v2_bookings (id, branch_id, request_id, request_hash, name, phone, member_id, type, \
             code, amount, rate, status, created_at, created_by, unit_size, published_rate_line_id) \
         VALUES ('BK-TIER', 'MAIN', 'req-tier', 'hash', 'Tier Member', '0800000001', 'MB-TIER', \
             'BUY', 'USD', 500, 31.70, 'PENDING', now(), 'member', 1, '{}'::uuid)",
        line_id
    ))
    .await;
}

async fn confirm(db: &TestDb) -> pos_error::AppResult<()> {
    let service = BookingService::new(
        BranchTx::new(Transactor::new(db.pool.clone())),
        Arc::new(PgBookingRepository::new(db.pool.clone())),
        Arc::new(PgMemberRepository::new(db.pool.clone())),
        Arc::new(PgBookingPorts::new(db.pool.clone())),
        Arc::new(UuidBookingIds),
    );
    let now = Utc::now();
    service
        .confirm(
            &BranchId::new("MAIN"),
            ConfirmBooking {
                id: "BK-TIER".to_string(),
                expires_at: Some(now + Duration::hours(2)),
                expected_rate: Some(dec("31.70")),
            },
            "owner",
            now,
        )
        .await
}

async fn status(db: &TestDb) -> Option<String> {
    db.text("SELECT status FROM v2_bookings WHERE id = 'BK-TIER'")
        .await
}

#[tokio::test]
async fn a_member_booking_made_at_a_tier_can_be_confirmed() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let ids = tier_usd(&db, &p.id).await;
    pending_booking(&db, id_of(&ids, "T2")).await;
    confirm(&db)
        .await
        .unwrap_or_else(|e| panic!("confirm refused: {e}"));
    assert_eq!(status(&db).await.as_deref(), Some("CONFIRMED"));
    db.finish().await;
}

#[tokio::test]
async fn a_booking_whose_tier_line_is_no_longer_current_is_not_confirmed() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let ids = tier_usd(&db, &p.id).await;
    pending_booking(&db, id_of(&ids, "T2")).await;
    // A new publication replaces the one the booking was quoted from.
    let q = db.publish("MAIN", &[USD]).await;
    tier_usd(&db, &q.id).await;
    assert!(confirm(&db).await.is_err(), "the rate moved");
    assert_eq!(status(&db).await.as_deref(), Some("PENDING"));
    db.finish().await;
}
