//! K-a caller ticket for `api/src/application/workflows/exchange/quote.rs` (#2063 series, after #2064 / PR 2074):
//! an overflowing quote line is refused as `validationFailed: quantity`, never a panic.
//! Written before the implementation by the ticket writer claude/b61d81bb on the xchange BE controller
//! claude/1656900d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `QuoteService::quote_with_coupon_in` computes THB with the unchecked `ExchangeArithmetic::thb` at three sites
//! (`quote.rs:223`, `:439`, `:566` on `main` `5f444eab`). `quantity × rate` past `Decimal::MAX` (about 7.9e28)
//! panics with `Multiplication overflowed`; the desk route then dies mid-request instead of answering. After the
//! fix every site uses `ExchangeArithmetic::try_thb` and maps `ArithmeticOverflow` to
//! `AppError::Domain("validationFailed: quantity …")`, which `v3/commands/quotes.rs` answers 400 `validationFailed`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod quote_common;

use std::str::FromStr;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use quote_common::*;
use rust_decimal::Decimal;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

/// USD × 32.10: `3e27 × 32.10 ≈ 9.6e28` is past `Decimal::MAX`. The quantity itself fits `Decimal` and has no
/// fraction, so it passes every check before the arithmetic.
const OVERFLOWING_USD: &str = "3000000000000000000000000000";

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-7".to_owned(),
        username: "user-employee-7".to_owned(),
        employee_id: Some("employee-7".to_owned()),
        role: Role::Front,
    }
}

fn line(currency: &str, side: Side, quantity: &str) -> QuoteRequestLine {
    QuoteRequestLine {
        currency: CurrencyCode::parse(currency).expect("a known currency"),
        shop_side: side,
        quantity: ForeignAmount::from_decimal(Decimal::from_str(quantity).expect("a decimal")),
        denomination_group: xc_rate::domain::currency::denomination_groups::GroupCode::all(),
        fee_thb: None,
    }
}

fn request(l: QuoteRequestLine) -> QuoteRequest {
    QuoteRequest {
        branch_id: BranchId::new("MAIN"),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines: vec![l],
    }
}

/// `quote_in` in its own transaction on a spawned task, committed whatever the answer. A panic inside is
/// returned as `Err(text)`, so the test can say "panicked" instead of dying.
async fn quote(
    db: &TestDb,
    req: QuoteRequest,
    now: DateTime<Utc>,
) -> Result<AppResult<Quote>, String> {
    let pool = db.pool.clone();
    let task = tokio::spawn(async move {
        let service = QuoteService::new(pool.clone());
        let mut tx = pool.begin().await.expect("begin");
        let answer = service.quote_in(&mut tx, req, &teller(), now).await;
        tx.commit().await.expect("commit");
        answer
    });
    match task.await {
        Ok(a) => Ok(a),
        Err(e) if e.is_panic() => {
            let p = e.into_panic();
            Err(p
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic".to_owned()))
        }
        Err(e) => Err(format!("task failed: {e}")),
    }
}

fn assert_quantity_refusal(answer: Result<AppResult<Quote>, String>, what: &str) {
    match answer {
        Err(panic) => panic!(
            "{what}: quote_in panicked ({panic}); want Err(Domain(\"validationFailed: quantity …\"))"
        ),
        Ok(Ok(q)) => panic!("{what}: quoted {}; want a refusal", q.id),
        Ok(Err(AppError::Domain(m))) => assert!(
            m.starts_with("validationFailed: quantity"),
            "{what}: want a message starting `validationFailed: quantity`, got {m:?}"
        ),
        Ok(Err(other)) => {
            panic!("{what}: want AppError::Domain(validationFailed: quantity …), got {other:?}")
        }
    }
}

/// Q1: a BUY whose `quantity × rate` overflows is refused as `validationFailed: quantity`; nothing is stored.
#[tokio::test]
async fn an_overflowing_buy_is_refused_as_quantity_and_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let before = db.quote_rows().await;

    let answer = quote(
        &db,
        request(line("USD", Side::Buy, OVERFLOWING_USD)),
        whole_second_now(),
    )
    .await;

    assert_quantity_refusal(answer, "overflowing BUY");
    assert_eq!(db.quote_rows().await, before, "nothing stored");
    db.finish().await;
}

/// Q2: the same for a SELL.
#[tokio::test]
async fn an_overflowing_sell_is_refused_as_quantity_and_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let before = db.quote_rows().await;

    let answer = quote(
        &db,
        request(line("USD", Side::Sell, OVERFLOWING_USD)),
        whole_second_now(),
    )
    .await;

    assert_quantity_refusal(answer, "overflowing SELL");
    assert_eq!(db.quote_rows().await, before, "nothing stored");
    db.finish().await;
}

/// Q3 (unchanged): an ordinary quantity is quoted with the same THB as before.
#[tokio::test]
async fn an_ordinary_quantity_is_quoted_as_before() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;

    let answer = quote(
        &db,
        request(line("USD", Side::Buy, "100")),
        whole_second_now(),
    )
    .await
    .expect("no panic")
    .expect("an ordinary quote");
    let thb: Decimal =
        sqlx::query_scalar("SELECT thb_amount FROM quote_lines WHERE quote_id = $1::uuid")
            .bind(answer.id.to_string())
            .fetch_one(&db.pool)
            .await
            .expect("the stored line");
    assert_eq!(thb, Decimal::from_str("3210.00").unwrap(), "100 × 32.10");
    db.finish().await;
}

/// Q4: no unchecked `ExchangeArithmetic::thb` / `foreign_for_thb` call is left in `quote.rs`. Sites `:439` and
/// `:566` sit behind `:223` for the same input, so only this scan pins them.
#[test]
fn quote_rs_has_no_unchecked_exchange_arithmetic() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/application/workflows/exchange/quote.rs");
    let text = std::fs::read_to_string(&path).expect("quote.rs");
    let hits: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.split("//").next().unwrap_or("")))
        .filter(|(_, code)| {
            [
                "ExchangeArithmetic::thb",
                "ExchangeArithmetic::foreign_for_thb",
            ]
            .iter()
            .any(|m| {
                code.match_indices(m).any(|(at, _)| {
                    !code[at + m.len()..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                })
            })
        })
        .map(|(n, code)| format!("quote.rs:{n}: {}", code.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "unchecked ExchangeArithmetic left:\n{}",
        hits.join("\n")
    );
}
