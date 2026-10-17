//! #2860: `complete.rs` uses checked `Money::try_minor()` on `line.settle_thb` and maps
//! overflow past `i64::MAX` hundredths to `AppError::Internal`, refusing the exchange
//! without writing or panicking.

mod quote_common;
mod tx4_common;
mod tx5_common;

use pos_error::AppError;
use quote_common::*;
use tx4_common::*;
use tx5_common::*;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::TransactionStatus;
use xc_kernel::money::Side;

#[tokio::test]
async fn complete_with_ordinary_figures_succeeds() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);

    let done = c
        .complete(&ledger, t.id, later(now, 5))
        .await
        .expect("an ordinary exchange completes normally");
    assert_eq!(done.transaction.id, t.id);
    assert_eq!(done.transaction.status, TransactionStatus::Completed);
}

#[tokio::test]
async fn complete_refuses_settle_thb_overflowing_i64() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);

    // i64::MAX is 9,223,372,036,854,775,807 hundredths (92,233,720,368,547,758.07 THB).
    // An amount with 9,223,372,036,854,775,808 hundredths (92,233,720,368,547,758.08 THB)
    // fits in numeric(20,2) but overflows i64 hundredths.
    sqlx::query(
        "UPDATE transaction_lines SET settle_thb = 92233720368547758.08 WHERE transaction_id = $1",
    )
    .bind(t.id.0)
    .execute(&c.desk.db.pool)
    .await
    .unwrap();

    let err = c
        .complete(&ledger, t.id, later(now, 5))
        .await
        .expect_err("settle_thb overflowing i64 must return AppError::Internal");

    match err {
        AppError::Internal(msg) => {
            assert!(
                msg.contains("settle_thb overflows i64"),
                "unexpected internal error message: {msg}"
            );
        }
        other => panic!("expected AppError::Internal, got: {other:?}"),
    }
}
