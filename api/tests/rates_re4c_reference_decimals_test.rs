//! Ticket #757 (RE-4c), must-fix 2 of the PR 902 review (claude/dell-claude-3, head `a13f3c1`):
//! the client's reference rate obeys the same decimal limit as the rate.
//! Written before the fix by the ticket author (claude/07d3765f, on the lane controller's order).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §7 [required]: a client rate field takes at most `8 + log10(unit_size)` decimals, because the
//! stored per-unit figure (`numeric(20,8)`) keeps 8. For JPY, per 100, that is 10.
//! - `"0.2301234567"` (10 places) × 100 = `23.01234567`: it fits, and is stored per unit.
//! - `"0.23012345678"` (11 places) × 100 = `23.012345678`: it does not fit. The save is refused
//!   with `TOO_MANY_DECIMALS` (`ทศนิยมเกินกำหนด`) and nothing is written. It is never stored as
//!   `NULL` (the review's probe measured exactly that at `a13f3c1`).
//!
//! The JPY line is published at 23.1 / 23.5 per 100. The desk sends a SELL of 1000 JPY at the
//! line's per-1 rate, 0.235, so only the reference decides the outcome.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod save_common;

use rust_decimal::Decimal;
use save_common::rates_re4c_common::{d, publish};
use save_common::*;

const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";

/// `MAIN` with an open shift, the customer of `person()`, a passport file and the JPY line.
async fn jpy_counter(shop: &Shop) -> (String, String) {
    shop.open_shift(MAIN, &[("THB", "100000"), ("JPY", "5000")])
        .await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    (customer, passport)
}

/// A LIVE cash SELL of 1000 JPY at the line's per-1 rate 0.235, with the per-1 `reference`.
fn jpy_sell(submission: &str, customer: &str, passport: &str, reference: &str) -> SaveTransaction {
    let mut cmd = form(submission, customer, passport);
    cmd.side = "SELL".to_owned();
    cmd.currency = "JPY".to_owned();
    cmd.amount = "1000".to_owned();
    cmd.rate = "0.235".to_owned();
    cmd.reference_rate = Some(reference.to_owned());
    cmd
}

#[tokio::test]
async fn a_reference_rate_beyond_the_units_places_is_too_many_decimals_and_writes_nothing() {
    let shop = Shop::start().await;
    let (customer, passport) = jpy_counter(&shop).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            jpy_sell("re4c-ref-11-places", &customer, &passport, "0.23012345678"),
            at(0),
        )
        .await;
    assert_eq!(
        refusal(refused),
        TOO_MANY_DECIMALS,
        "11 places per 1 is 9 places per 100"
    );
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_reference_rate_at_the_units_limit_is_stored_per_unit() {
    let shop = Shop::start().await;
    let (customer, passport) = jpy_counter(&shop).await;

    let saved = shop
        .save(
            MAIN,
            jpy_sell("re4c-ref-10-places", &customer, &passport, "0.2301234567"),
            at(0),
        )
        .await
        .expect("10 places per 1 is 8 places per 100: the save is accepted");
    let reference: Option<Decimal> =
        sqlx::query_scalar("SELECT l.reference_rate FROM transaction_lines l JOIN transactions t ON t.id = l.transaction_id WHERE t.document_ref = $1")
            .bind(&saved.document_ref)
            .fetch_one(&shop.pool)
            .await
            .expect("read the exchange");
    assert_eq!(
        reference,
        Some(d("23.01234567")),
        "stored per 100, all 8 places"
    );
    shop.finish().await;
}
