//! #2852 / #2063: checked arithmetic for kiosk_repository.rs.
//!
//! Verifies that `insert_in` refuses money values whose hundredths overflow `i64::MAX`,
//! returning `AppError::Internal` instead of silently wrapping or storing corrupted amounts.
//!
//! `money_to` is reachable through `thb` (Buy: amount ≤ 1e7 × rate ≤ 1e8 ÷ unit_size ≥ 0.0001,
//! up to 1e19 THB against an i64 limit of ≈9.22e16 THB). `foreign` and `input_amount` cannot
//! overflow: `kiosk_machine.rs:43` caps the input at 1e9 hundredths, Sell `foreign` is checked
//! at `:73-78`, and the reader `money_from` (`kiosk_repository.rs:173-180`) bounds stored amounts
//! to i64.

mod kiosk_service_common;

use kiosk_service_common::*;
use pos_error::AppError;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::entities::kiosk_request::KioskRequest;
use xc_kiosk::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use xc_kiosk::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use xc_kiosk::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use xc_kiosk::domain::kiosk::value_objects::kiosk_time::KioskTime;
use xc_kiosk::domain::kiosk::value_objects::payment_method::PaymentMethod;
use xc_kiosk::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

fn overflowing_money() -> Money {
    // (1 << 64) + 100 = 18_446_744_073_709_551_716 hundredths.
    // In i64, this silently wraps to 100 hundredths (1.00).
    Money::try_from_minor_i128((1i128 << 64) + 100).expect("valid Decimal at scale 2")
}

fn dummy_request(id: &str, foreign: Money, thb: Money, input_amount: Money) -> KioskRequest {
    KioskRequest {
        id: id.to_owned(),
        status: KioskStatus::Quoted,
        created: KioskTime::from_epoch_millis(T0),
        staff: None,
        quote: KioskQuote {
            side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            rate: Rate::parse("35.00").unwrap(),
            unit_size: UnitSize::parse("1").unwrap(),
            line_id: None,
            foreign,
            thb,
            input_amount,
            payment_method: PaymentMethod::Cash,
        },
        transaction: None,
    }
}

#[tokio::test]
async fn test_thb_overflow_refused() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    let branch = BranchId::new(MAIN);

    let req = dummy_request(
        "req-thb-overflow",
        Money::from_minor(100).unwrap(),
        overflowing_money(),
        Money::from_minor(100).unwrap(),
    );

    let mut tx = db.begin().await;
    let res = repo.insert_in(&mut tx, &branch, &req, "127.0.0.1").await;
    assert!(
        matches!(res, Err(AppError::Internal(_))),
        "expected AppError::Internal for overflowing thb, got: {res:?}"
    );
    drop(tx);

    db.finish().await;
}

#[tokio::test]
async fn test_ordinary_figures_accepted() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    let branch = BranchId::new(MAIN);

    let req = dummy_request(
        "req-ordinary",
        Money::from_minor(10000).unwrap(),
        Money::from_minor(350000).unwrap(),
        Money::from_minor(10000).unwrap(),
    );

    let mut tx = db.begin().await;
    let res = repo.insert_in(&mut tx, &branch, &req, "127.0.0.1").await;
    assert!(res.is_ok(), "ordinary figures must succeed: {res:?}");
    tx.commit().await.unwrap();

    let found = repo.find(&branch, "req-ordinary").await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.quote.foreign, Money::from_minor(10000).unwrap());
    assert_eq!(found.quote.thb, Money::from_minor(350000).unwrap());
    assert_eq!(found.quote.input_amount, Money::from_minor(10000).unwrap());

    db.finish().await;
}
