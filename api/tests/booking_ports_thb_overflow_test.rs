//! Up-front tests for the K-a caller migration of `PgBookingPorts::counter_rate_in` (#2063,
//! ticket B), written by the ticket author (claude/d7aa64) before the fix. Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! `infrastructure/bookings/booking_ports.rs:138` prices the booking's amount at the current line
//! with `ExchangeArithmetic::thb` (amount × rate ÷ unit_size), whose `/` panics when the value
//! leaves `Decimal` (≈7.9e28). The route cannot get there — `POST /api/bookings` parses the amount
//! with `Money::parse` (`http/bookings.rs:368`), capped at 1e8 — so, per the ticket author's
//! ruling, the port is tested on its own: an amount of `i64::MAX` hundredths (≈9.2e16, what the
//! port's own `Decimal::new(minor as i64, 2)` still holds) at a line of 1e8 per 0.0001 is
//! 9.2e16 × 1e8 ÷ 0.0001 ≈ 9.2e28, past `Decimal::MAX`.
//!
//! Required: the checked `ExchangeArithmetic::try_thb` (#2064), its overflow answered as the
//! booking refusal the route already turns into a 400 — `AppError::Domain(sentence)`
//! (`create.rs:64-66` propagates the port's error with `?`; `http/bookings.rs:412` maps
//! `Err(AppError::Domain(sentence))` to `refuse(400, &sentence)`), with the sentence
//! `BookingError::AmountOutOfRange` ("ยอดเงินสูงเกินขอบเขตระบบ", `booking_error.rs`), the
//! existing refusal for an amount past what the system holds. Never a panic.
//!
//! Keep-pins: an ordinary amount at an ordinary line, and a small amount at the extreme line,
//! still answer the line as today.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod compat_common;
mod rates_re4_common;
mod snapshot_common;

use pos_error::AppError;
use snapshot_common::{TestDb, branch};
use sqlx::{Postgres, Transaction};
use xc_exchange::domain::booking::repositories::booking_ports::{BookingPorts, CounterLine};
use xc_exchange::domain::booking::value_objects::booking_error::BookingError;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};
use xchange::infrastructure::bookings::booking_ports::PgBookingPorts;

const MAIN: &str = "MAIN";

struct Shop {
    db: TestDb,
    ports: Box<dyn BookingPorts>,
}

impl Shop {
    /// `MAIN` with USD published at 32.1 / 32.6 per 1.
    async fn open() -> Shop {
        let db = TestDb::start().await;
        rates_re4_common::publish(&db.pool, MAIN, &[("USD", "32.1", "32.6")]).await;
        let ports: Box<dyn BookingPorts> = Box::new(PgBookingPorts::new(db.pool.clone()));
        Shop { db, ports }
    }

    /// MAIN's current USD line moved to 1e8 per 0.0001 (`Rate::parse`'s largest rate, the
    /// smallest unit `numeric(20,4)` holds).
    async fn extreme_line(&self) {
        let done = sqlx::query(
            "UPDATE published_rate_lines l SET buy_rate = 100000000, sell_rate = 100000000, \
                 unit_size = 0.0001 \
             FROM branches b \
             WHERE b.id = $1 AND l.publication_id = b.current_publication_id \
               AND l.currency_code = 'USD'",
        )
        .bind(MAIN)
        .execute(&self.db.pool)
        .await
        .expect("move the line");
        assert!(done.rows_affected() >= 1, "MAIN has a USD line");
    }

    async fn begin(&self) -> Transaction<'static, Postgres> {
        self.db.pool.begin().await.expect("begin a transaction")
    }

    async fn counter(
        &self,
        side: Side,
        amount: Money,
    ) -> pos_error::AppResult<Option<CounterLine>> {
        let usd = CurrencyCode::parse("USD").expect("a currency");
        let mut tx = self.begin().await;
        let got = self
            .ports
            .counter_rate_in(&mut tx, &branch(MAIN), &usd, side, Some(amount))
            .await;
        tx.rollback().await.expect("rollback");
        got
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

/// `i64::MAX` hundredths: the largest amount the port's `Decimal::new(minor as i64, 2)` holds.
fn largest_amount() -> Money {
    Money::try_from_minor_i128(i64::MAX as i128).expect("i64::MAX hundredths is an amount")
}

fn assert_out_of_range(got: pos_error::AppResult<Option<CounterLine>>, label: &str) {
    match got {
        Err(AppError::Domain(sentence)) => assert_eq!(
            sentence,
            BookingError::AmountOutOfRange.to_string(),
            "{label}: the refusal sentence"
        ),
        other => panic!("{label}: expected Err(AppError::Domain(AmountOutOfRange)), got {other:?}"),
    }
}

#[tokio::test]
async fn a_buy_amount_past_decimal_at_the_line_is_refused_not_a_panic() {
    let shop = Shop::open().await;
    shop.extreme_line().await;

    let got = shop.counter(Side::Buy, largest_amount()).await;
    assert_out_of_range(got, "BUY");

    shop.finish().await;
}

#[tokio::test]
async fn a_sell_amount_past_decimal_at_the_line_is_refused_not_a_panic() {
    let shop = Shop::open().await;
    shop.extreme_line().await;

    let got = shop.counter(Side::Sell, largest_amount()).await;
    assert_out_of_range(got, "SELL");

    shop.finish().await;
}

#[tokio::test]
async fn an_ordinary_amount_still_answers_the_line() {
    let shop = Shop::open().await;
    let usd = rates_re4_common::line(&shop.db.pool, MAIN, "USD").await;

    let amount = Money::parse("100000000").expect("the largest input amount");
    for (side, want) in [(Side::Buy, "32.1"), (Side::Sell, "32.6")] {
        let got = shop
            .counter(side, amount)
            .await
            .unwrap_or_else(|e| panic!("{side:?}: {e:?}"))
            .unwrap_or_else(|| panic!("{side:?}: MAIN has a USD line"));
        assert_eq!(
            got.line_id.to_string(),
            usd.id,
            "{side:?}: the current line"
        );
        assert_eq!(got.rate, Rate::parse(want).expect("a rate"), "{side:?}");
        assert_eq!(got.unit_size, UnitSize::ONE, "{side:?}");
    }

    shop.finish().await;
}

#[tokio::test]
async fn a_small_amount_at_the_extreme_line_still_answers_the_line() {
    let shop = Shop::open().await;
    shop.extreme_line().await;

    // 1.00 × 1e8 ÷ 0.0001 = 1e12: inside Decimal, so no refusal.
    let got = shop
        .counter(Side::Buy, Money::parse("1.00").expect("an amount"))
        .await
        .unwrap_or_else(|e| panic!("BUY 1.00: {e:?}"))
        .expect("MAIN has a USD line");
    assert_eq!(got.rate, Rate::parse("100000000").expect("a rate"));
    assert_eq!(got.unit_size, UnitSize::parse("0.0001").expect("a unit"));

    shop.finish().await;
}
