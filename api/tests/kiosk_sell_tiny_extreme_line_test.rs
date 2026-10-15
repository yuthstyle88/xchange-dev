//! A kiosk SELL quote whose foreign amount passes what the desk can hold is refused as an invalid
//! amount: never a 500, and never a stored request whose foreign amount differs from the one the
//! kiosk computed and answered. Up-front tests written by the ticket author (claude/d7aa64) before
//! the fix. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The chain, measured on `origin/impl/2065-try-minor` `d905ce39` (kiosk code identical to
//! `origin/main` `104e817d`):
//! 1. `KioskMachine::quote` (`crates/xc-kiosk/src/domain/kiosk/entities/kiosk_machine.rs:55-70`)
//!    caps the SELL input (baht, at most 10,000,000.00) but not the foreign amount it derives:
//!    `Money::try_from_minor_i128` bounds it only by what `Decimal` holds at scale 2. On a line of
//!    rate 0.00000001 per 9999999999999999.9999, 0.01 THB derives 9999999999999999999900.00 USD.
//! 2. `Money::minor()` (`crates/xc-kernel/src/money/money.rs:42-46`, `mantissa() as i64`) wraps
//!    it silently to 2003764205206886640 hundredths, at `customer.rs:85` (the tier lookup),
//!    `customer.rs:155` (the quantity asked of the quote service) and
//!    `infrastructure/kiosk/kiosk_repository.rs:183` (`money_to`, the request's
//!    `foreign_amount`).
//! 3. The quote service writes the wrapped 20037642052068866.40 into `quote_lines.quantity`
//!    `numeric(20,4)`: Postgres 22003 `numeric field overflow` (precision 20, scale 4 must be
//!    below 10^16), and the kiosk answers 500.
//!
//! When the wrapped figure happens to fit (rate 0.00000001 per 1000, 1,844,674.41 THB derives
//! 184467441000000000.00 USD, which wraps to 26290448384 hundredths) nothing fails: the kiosk
//! answers 200 with `foreign` 184467441000000000 while the committed request holds
//! `foreign_amount` 262904483.84 and its quote line `thb_amount` 0.00. That is a wrong amount
//! stored, not only a 500.
//!
//! Required: both are refused exactly as any invalid amount is (`KioskError::AmountOrCurrencyInvalid`,
//! the same 400 and sentence as an amount of 0) and write neither a request nor a quote. Where
//! the ceiling sits between "fits 64-bit hundredths" and the kiosk's own 10,000,000.00 is not
//! decided here; both rows exceed every candidate, so no boundary row is pinned.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::Counter;
use serde_json::{Value, json};
use xc_kernel::money::{CurrencyCode, MinorUnit, Money, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::commands::quote_input::QuoteInput;
use xc_kiosk::domain::kiosk::entities::kiosk_machine::KioskMachine;
use xc_kiosk::domain::kiosk::value_objects::counter_facts::{CounterFacts, CounterLine, LineId};
use xc_kiosk::domain::kiosk::value_objects::kiosk_error::KioskError;
use xc_kiosk::domain::kiosk::value_objects::kiosk_time::KioskTime;

const MAIN: &str = "MAIN";

/// `MAIN`'s dollar re-sized to `unit_size` and published at `rate` both ways.
async fn widen_the_dollar(counter: &Counter, unit_size: &str, rate: &str) {
    sqlx::query("UPDATE currencies SET unit_size = $1::numeric WHERE code = 'USD'")
        .bind(unit_size)
        .execute(&counter.desk.db.pool)
        .await
        .expect("re-size the dollar");
    kiosk_http_common::rates_re4_common::publish(
        &counter.desk.db.pool,
        MAIN,
        &[("USD", rate, rate)],
    )
    .await;
}

async fn sell(counter: &Counter, amount: Value) -> rpc_common::Answer {
    counter
        .ask(
            None,
            "quote",
            json!({ "type": "SELL", "currency": "USD", "amount": amount }),
        )
        .await
}

async fn stored(counter: &Counter) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT k.foreign_amount::text, k.thb::text, coalesce(l.quantity::text, 'NULL') \
         FROM kiosk_requests k LEFT JOIN quote_lines l ON l.quote_id = k.quote_id \
         ORDER BY k.created_at",
    )
    .fetch_all(&counter.desk.db.pool)
    .await
    .expect("read the stored requests")
}

async fn quotes(counter: &Counter) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM quotes")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("count the quotes")
}

/// The answer to a SELL of 0, the kiosk's refusal of an invalid amount (`AmountOrCurrencyInvalid`).
async fn invalid_amount_refusal(counter: &Counter) -> (u16, String) {
    let answer = sell(counter, json!(0)).await;
    assert_eq!(answer.status, 400, "a SELL of 0: {}", answer.text);
    (answer.status, answer.text)
}

/// Asserts `amount` is refused as an invalid amount and leaves neither a request nor a quote.
async fn assert_refused_and_nothing_stored(counter: &Counter, amount: Value, what: &str) {
    let answer = sell(counter, amount).await;
    let rows = stored(counter).await;
    assert!(
        rows.is_empty(),
        "{what}: nothing may be stored, but the request holds (foreign_amount, thb, quote line \
         quantity) {rows:?} while the kiosk answered {} {}",
        answer.status,
        answer.text
    );
    assert_eq!(quotes(counter).await, 0, "{what}: no quote is left behind");
    let refusal = invalid_amount_refusal(counter).await;
    assert_eq!(
        (answer.status, answer.text),
        refusal,
        "{what}: refused as an invalid amount (400, the sentence of an amount of 0), not a 500"
    );
}

#[actix_web::test]
async fn a_tiny_sell_on_a_line_widened_to_the_extremes_is_refused_not_a_500() {
    let counter = Counter::open(&[]).await;
    widen_the_dollar(&counter, "9999999999999999.9999", "0.00000001").await;
    // 0.01 THB / 0.00000001 × 9999999999999999.9999 = 9999999999999999999900.00 USD
    assert_refused_and_nothing_stored(&counter, json!(0.01), "SELL 0.01 THB at 1e-8 per ~1e16")
        .await;
    counter.finish().await;
}

#[actix_web::test]
async fn a_sell_whose_foreign_wraps_past_64_bits_never_stores_a_different_amount() {
    let counter = Counter::open(&[]).await;
    widen_the_dollar(&counter, "1000", "0.00000001").await;
    // 1,844,674.41 THB / 0.00000001 × 1000 = 184467441000000000.00 USD; its hundredths pass
    // i64::MAX and `minor()` wraps them to 26290448384 (262904483.84 USD).
    assert_refused_and_nothing_stored(
        &counter,
        json!(1844674.41),
        "SELL 1,844,674.41 THB at 1e-8 per 1000",
    )
    .await;
    counter.finish().await;
}

#[actix_web::test]
async fn an_ordinary_sell_is_still_quoted_and_stored_as_answered() {
    let counter = Counter::open(&[]).await;
    // MAIN's dollar at SELL 32.6 per 1 (the counter's fixture)
    for (amount, foreign, thb) in [
        // 1,000 THB / 32.6 = 30.674… cut to 30.67; 30.67 × 32.6 = 999.84
        (json!(1000), "30.67", "999.84"),
        // the largest baht the kiosk takes: 10,000,000 / 32.6 = 306748.466… cut to 306748.46;
        // 306748.46 × 32.6 = 9999999.796 → 9999999.80 (the answer's shortest number: 9999999.8)
        (json!(10000000), "306748.46", "9999999.8"),
    ] {
        let answer = sell(&counter, amount.clone()).await;
        assert_eq!(answer.status, 200, "SELL {amount}: {}", answer.text);
        let body = answer.json();
        assert_eq!(
            body["foreign"].to_string(),
            foreign,
            "SELL {amount}: foreign"
        );
        assert_eq!(body["thb"].to_string(), thb, "SELL {amount}: thb");
    }
    assert_eq!(
        stored(&counter).await,
        [
            ("30.67", "999.84", "30.6700"),
            ("306748.46", "9999999.80", "306748.4600"),
        ]
        .map(|(f, t, q)| (f.to_owned(), t.to_owned(), q.to_owned())),
        "each request and its quote line hold the foreign amount the kiosk answered"
    );
    counter.finish().await;
}

/// The creator of the figure: the machine itself refuses a SELL whose derived foreign amount
/// cannot be held as 64-bit hundredths, before anything reads it through `Money::minor()`.
#[test]
fn the_machine_refuses_a_sell_whose_foreign_passes_64_bit_hundredths() {
    let rate = Rate::parse("0.00000001").expect("rate");
    let line = |unit: &str| CounterLine {
        line_id: LineId(1),
        rate,
        unit_size: UnitSize::parse(unit).unwrap_or_else(|e| panic!("unit {unit}: {e}")),
        minor_unit: MinorUnit::TWO,
    };
    let quote = |unit: &str, baht: &str| {
        let input = QuoteInput {
            side: Some(Side::Sell),
            currency: Some(CurrencyCode::parse("USD").expect("USD")),
            amount: Some(Money::parse(baht).unwrap_or_else(|e| panic!("baht {baht}: {e}"))),
            payment_method: None,
        };
        let facts = CounterFacts {
            open: true,
            line: Some(line(unit)),
        };
        KioskMachine::quote(&input, &facts, "K-1", KioskTime::from_epoch_millis(0))
    };
    for (unit, baht) in [("9999999999999999.9999", "0.01"), ("1000", "1844674.41")] {
        assert_eq!(
            quote(unit, baht).map(|r| r.quote.foreign),
            Err(KioskError::AmountOrCurrencyInvalid),
            "SELL {baht} THB at 0.00000001 per {unit}"
        );
    }
    // keep: on the same rate a figure that fits is still priced, cut to two places
    let fits = quote("1", "0.01").expect("0.01 THB at 0.00000001 per 1 is priced");
    assert_eq!(fits.quote.foreign, Money::parse("1000000").expect("money"));
}
