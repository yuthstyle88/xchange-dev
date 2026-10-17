//! K-b caller migration for `api/src/application/workflows/exchange/quote.rs` (#2063 / #2861):
//! an overflowing settle amount past `i64::MAX` satang is refused as `AppError::Domain("invalid settle amount")`,
//! never wrapping silently into a corrupted positive settle figure or panicking.
//!
//! Written for ticket #2861 by worker gemini/pool-1.
//!
//! `QuoteService::quote_with_coupon_in` converts `Money` amounts to satang using `.try_minor()`
//! across all 11 sites (initial quote calculation and coupon benefit recomputation).
//! When THB satang exceeds `i64::MAX` (e.g. `(1u128 << 64) + 321000` satang = 184,467,440,737,098,726.16 THB),
//! unchecked `.minor()` truncates `i128` to `i64`, wrapping silently to `+321000` satang (3,210.00 THB).
//! With `.try_minor()`, the operation returns `NumberError::Invalid`, which is mapped to
//! `AppError::Domain("invalid settle amount".to_string())`.
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

/// Quantity whose satang `(1u128 << 64) + 321_000` satang = `184467440737098726.16` THB.
/// At rate 1.0, this does not overflow `Decimal` in `ExchangeArithmetic::try_thb`, but wraps
/// silently past `i64::MAX` satang to `+321000` satang (3,210.00 THB) when converted with `.minor()`.
const OVERFLOWING_SATANG_QUANTITY: &str = "184467440737098726.16";

const USD_ONE_TO_ONE: Line = Line::manual("USD", "1", "1.00", "1.00");

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

fn assert_invalid_settle_refusal(answer: Result<AppResult<Quote>, String>, what: &str) {
    match answer {
        Err(panic) => panic!(
            "{what}: quote_in panicked ({panic}); want Err(Domain(\"invalid settle amount\"))"
        ),
        Ok(Ok(q)) => panic!(
            "{what}: quoted {} with settle {} THB; want a refusal",
            q.id, q.lines[0].settle_thb
        ),
        Ok(Err(AppError::Domain(m))) => assert_eq!(
            m, "invalid settle amount",
            "{what}: want AppError::Domain(\"invalid settle amount\"), got {m:?}"
        ),
        Ok(Err(other)) => {
            panic!("{what}: want AppError::Domain(\"invalid settle amount\"), got {other:?}")
        }
    }
}

/// A BUY whose satang amount overflows `i64::MAX` is refused as `invalid settle amount`,
/// rather than wrapping to +3210.00 THB; nothing is stored.
#[tokio::test]
async fn an_overflowing_satang_buy_is_refused_and_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD_ONE_TO_ONE]).await;
    let before = db.quote_rows().await;

    let answer = quote(
        &db,
        request(line("USD", Side::Buy, OVERFLOWING_SATANG_QUANTITY)),
        whole_second_now(),
    )
    .await;

    assert_invalid_settle_refusal(answer, "overflowing BUY satang");
    assert_eq!(db.quote_rows().await, before, "nothing stored");
    db.finish().await;
}

/// A SELL whose satang amount overflows `i64::MAX` is refused as `invalid settle amount`,
/// rather than wrapping to +3210.00 THB; nothing is stored.
#[tokio::test]
async fn an_overflowing_satang_sell_is_refused_and_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD_ONE_TO_ONE]).await;
    let before = db.quote_rows().await;

    let answer = quote(
        &db,
        request(line("USD", Side::Sell, OVERFLOWING_SATANG_QUANTITY)),
        whole_second_now(),
    )
    .await;

    assert_invalid_settle_refusal(answer, "overflowing SELL satang");
    assert_eq!(db.quote_rows().await, before, "nothing stored");
    db.finish().await;
}

/// An ordinary quantity is quoted with the expected settle THB as before.
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
    let settle: Decimal =
        sqlx::query_scalar("SELECT settle_thb FROM quote_lines WHERE quote_id = $1::uuid")
            .bind(answer.id.to_string())
            .fetch_one(&db.pool)
            .await
            .expect("the stored line");
    assert_eq!(
        settle,
        Decimal::from_str("3210.00").unwrap(),
        "100 × 32.10 = 3210.00"
    );
    db.finish().await;
}

/// No unchecked `Money::minor()` call is left in `quote.rs`.
#[test]
fn quote_rs_has_no_unchecked_money_minor() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/application/workflows/exchange/quote.rs");
    let text = std::fs::read_to_string(&path).expect("quote.rs");
    let hits: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.split("//").next().unwrap_or("")))
        .filter(|(_, code)| {
            code.match_indices(".minor()").any(|(at, _)| {
                !code[at + ".minor()".len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
            })
        })
        .map(|(n, code)| format!("quote.rs:{n}: {}", code.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "unchecked Money::minor left:\n{}",
        hits.join("\n")
    );
}
