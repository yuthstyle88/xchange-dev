//! Tests for #2063 hardening in PgSettingsRepository (`repository.rs`).
//!
//! Verifies that approval limit updates reject money amounts whose hundredths
//! exceed `i64::MAX` instead of wrapping or panicking.

#[allow(unused_imports)]
mod common;

use chrono::Utc;
use common::TestDb;
use pos_error::AppError;
use uuid::Uuid;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

#[tokio::test]
async fn set_approval_limit_refuses_overflowing_amount() {
    let db = TestDb::start().await;
    let repo = PgSettingsRepository::new(db.pool.clone());
    let org_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let branch_id = BranchId::new("MAIN");
    let now = Utc::now();

    let mut tx = db.pool.begin().await.expect("begin tx");

    // (u64::MAX + 101) hundredths exceeds i64::MAX.
    // Unchecked minor() truncates/casts i128 to i64, yielding 100 > 0.
    // try_minor() returns NumberError::Invalid.
    let huge = Money::try_from_minor_i128((u64::MAX as i128) + 101).expect("create huge money");
    assert_eq!(huge.minor(), 100, "unchecked minor wraps to positive 100");
    assert!(huge.try_minor().is_err(), "try_minor refuses overflow");

    let err = repo
        .set_approval_limit_in(&mut tx, org_id, &branch_id, Some(huge), "test-user", now)
        .await
        .expect_err("overflowing limit must be refused");

    match err {
        AppError::Domain(msg) => assert_eq!(
            msg,
            "validationFailed: approvalLimitThb must be greater than zero"
        ),
        other => panic!("expected AppError::Domain, got {other:?}"),
    }

    // Ordinary limit succeeds and reads back
    let valid = Money::parse("50000.00").expect("valid money");
    repo.set_approval_limit_in(&mut tx, org_id, &branch_id, Some(valid), "test-user", now)
        .await
        .expect("valid limit succeeds");

    let read_back = repo
        .approval_limit_in(&mut tx, org_id, &branch_id)
        .await
        .expect("read approval limit");
    assert_eq!(read_back, Some(valid));

    tx.rollback().await.expect("rollback");
    db.finish().await;
}
