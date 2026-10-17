//! Tests for #2063 checked money arithmetic in `customer.rs`.
//!
//! K-b site: `customer.rs:241` (`request.quote.foreign.try_minor()`).
//!
//! Replaces unchecked `Money::minor()` with `Money::try_minor()` in `confirm_with`.
//!
//! These tests verify:
//! 1. Ordinary quotes confirm successfully with their published counter and tier rates.
//! 2. Stored requests with an overflowing foreign amount (past i64 hundredths) cannot
//!    silently wrap into a valid amount or panic, and are refused with an error.

mod kiosk_service_common;

use kiosk_service_common::*;

#[tokio::test]
async fn ordinary_quote_confirms_successfully() {
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    let service = db.service(ports);
    let main = branch(MAIN);

    service
        .quote(&main, buy100("q1"), at(0))
        .await
        .expect("quote succeeded");

    let confirmed = service
        .confirm(&main, "q1", at(4000))
        .await
        .expect("confirm succeeded");
    assert_eq!(confirmed.status, KioskStatus::Waiting);

    db.finish().await;
}

#[tokio::test]
async fn a_stored_request_whose_foreign_amount_overflows_is_refused_at_confirm() {
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    let service = db.service(ports);
    let main = branch(MAIN);

    // Seed a standard quote first so all related records/rates exist
    service
        .quote(&main, buy100("q_overflow"), at(0))
        .await
        .expect("quote succeeded");

    // Widen column to hold an amount past i64 hundredths (2^64 + 100,000 hundredths)
    sqlx::query("ALTER TABLE kiosk_requests ALTER COLUMN foreign_amount TYPE numeric(38, 2)")
        .execute(&db.pool)
        .await
        .expect("widen foreign_amount column");

    // 2^64 + 100,000 hundredths = 184467440737096516.16
    // In unchecked minor(), this would wrap to 100_000 (1,000.00).
    sqlx::query(
        "UPDATE kiosk_requests SET foreign_amount = 184467440737096516.16 WHERE id = 'q_overflow'",
    )
    .execute(&db.pool)
    .await
    .expect("update with overflowing amount");

    let result = service.confirm(&main, "q_overflow", at(4000)).await;
    assert!(
        result.is_err(),
        "confirm must refuse an overflowing amount instead of silently wrapping"
    );

    db.finish().await;
}
