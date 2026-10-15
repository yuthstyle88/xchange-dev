//! TX-9 #677 prerequisite — a save writes the slip once, on `transaction_payments.slip_file_key`,
//! and no longer a `transaction_files` row of kind `SLIP` (BE controller's ruling, #677
//! issuecomment-6183687076). Written before the implementation; do not edit, delete or weaken.
//!
//! 1. `a_compat_save_with_a_slip_writes_no_slip_row` — red on main (the compat writer pushes a SLIP row,
//!    `api/crates/xc-exchange/src/infrastructure/exchange/transaction_repository.rs:341-343`).
//! 2. `a_compat_cash_save_writes_passport_and_receipt_only` — green on main, a pin.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod save_common;

use save_common::*;

/// The `transaction_files` kinds of the exchange saved with `submission`, sorted, as `KIND=key`.
async fn files_of(shop: &Shop, submission: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT f.kind || '=' || f.file_key FROM transaction_files f \
         JOIN transactions t ON t.id = f.transaction_id \
         WHERE t.submission_id = $1 ORDER BY f.kind",
    )
    .bind(submission)
    .fetch_all(&shop.pool)
    .await
    .expect("read transaction_files")
}

async fn slip_on_payment(shop: &Shop, submission: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT p.slip_file_key FROM transaction_payments p \
         JOIN transactions t ON t.id = p.transaction_id \
         WHERE t.submission_id = $1",
    )
    .bind(submission)
    .fetch_one(&shop.pool)
    .await
    .expect("one payment for the saved exchange")
}

fn kinds(files: &[String]) -> Vec<&str> {
    files
        .iter()
        .map(|f| f.split('=').next().unwrap_or(""))
        .collect()
}

// 1.
#[tokio::test]
async fn a_compat_save_with_a_slip_writes_no_slip_row() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;

    let submission = "tx9-slip-0001-aaaaaaaaaaaa";
    let mut cmd = form(submission, &customer, &passport);
    cmd.method = "Bank Transfer".to_owned();
    cmd.payment_ref = "TRX-1".to_owned();
    cmd.slip = Some(slip.clone());
    shop.save(MAIN, cmd, at(10))
        .await
        .expect("a transfer with a slip saves");

    assert_eq!(
        slip_on_payment(&shop, submission).await.as_deref(),
        Some(slip.as_str()),
        "the slip's one home is the payment"
    );
    let files = files_of(&shop, submission).await;
    assert!(
        !kinds(&files).contains(&"SLIP"),
        "TX-9 #677 ruling: the slip lives on the payment only, no SLIP row: {files:?}"
    );
    assert!(
        kinds(&files).contains(&"PASSPORT") && kinds(&files).contains(&"RECEIPT"),
        "the PASSPORT and RECEIPT rows stay: {files:?}"
    );

    shop.finish().await;
}

// 2.
#[tokio::test]
async fn a_compat_cash_save_writes_passport_and_receipt_only() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;

    let submission = "tx9-slip-0002-aaaaaaaaaaaa";
    shop.save(MAIN, form(submission, &customer, &passport), at(10))
        .await
        .expect("a cash save");

    assert_eq!(slip_on_payment(&shop, submission).await, None);
    let files = files_of(&shop, submission).await;
    assert_eq!(kinds(&files), ["PASSPORT", "RECEIPT"], "{files:?}");
    assert!(
        files.contains(&format!("PASSPORT={passport}")),
        "the passport row keeps the uploaded key: {files:?}"
    );

    shop.finish().await;
}
