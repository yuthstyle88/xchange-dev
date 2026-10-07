//! Written before the implementation (ticket #276). Do not
//! edit, delete or weaken.
//!
//! A saved exchange is read back with the figures the save wrote. The save
//! writes what `Quote::compute` produced (`principal`, `received`, `paid`), and a
//! computed baht figure may exceed the 100,000,000 a person may type
//! (`xc-kernel/src/money/money.rs:21-25`; v2 stores 3,209,999,999.68 for
//! 99,999,999.99 × 32.1). So `find`, `list` and the repeat-save lookup must
//! accept every figure the save can write — the input cap is not a rule for a
//! stored value. A value the save can never write (negative) is still a fault.
//!
//! The largest figure a save can write: the form refuses a principal above
//! 90,071,992,547,409.91 (`transaction_form.rs:147-152`, v2 fixture
//! `gen_save_form.mjs`), and a fee is at most 100,000,000.
//!
//! Needs `TEST_DATABASE_URL` (see `save_common/mod.rs`); nothing here skips.

mod save_common;

use pos_error::AppError;
use save_common::*;

/// The three baht figures of a stored exchange, as the reader answers them.
fn figures(t: &Transaction) -> (String, String, String) {
    (
        t.principal.to_string(),
        t.received.to_string(),
        t.paid.to_string(),
    )
}

fn owned(p: &str, r: &str, d: &str) -> (String, String, String) {
    (p.to_owned(), r.to_owned(), d.to_owned())
}

async fn found(shop: &Shop, document_ref: &str) -> Transaction {
    shop.transactions
        .find(&branch(MAIN), document_ref)
        .await
        .unwrap_or_else(|e| panic!("find {document_ref}: {e:?}"))
        .unwrap_or_else(|| panic!("{document_ref} is kept"))
}

/// A sale of `amount` dollars at the counter's sell rate `rate`, cash.
fn sale(
    submission: &str,
    customer: &str,
    passport: &str,
    amount: &str,
    rate: &str,
) -> SaveTransaction {
    let mut cmd = form(submission, customer, passport);
    cmd.side = "SELL".to_owned();
    cmd.amount = amount.to_owned();
    cmd.rate = rate.to_owned();
    cmd
}

#[tokio::test]
async fn a_sale_whose_baht_is_past_the_input_cap_is_read_back_whole() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "0"), ("USD", "100000000")]).await;
    let mut cmd = sale(
        "large-0001-aaaaaaaaaa",
        &customer,
        &passport,
        "99999999.99",
        "33",
    );
    cmd.fee = "10".to_owned();
    let saved = shop.save(MAIN, cmd, at(0)).await.expect("the save");

    let stored = found(&shop, &saved.document_ref).await;
    assert_eq!(
        figures(&stored),
        owned("3299999999.67", "3300000009.67", "0.00")
    );
    assert_eq!(stored.amount.to_string(), "99999999.99");
    assert_eq!(stored.fee.to_string(), "10.00");
    // the list of the branch answers it too (the compat snapshot reads this list)
    let listed = shop
        .transactions
        .list(&branch(MAIN), Mode::Live)
        .await
        .expect("list the branch");
    assert_eq!(listed, vec![stored]);
    shop.finish().await;
}

#[tokio::test]
async fn the_largest_figures_a_save_can_write_are_read_back() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "0"), ("USD", "100000000")]).await;
    // the largest principal the form accepts, plus the largest fee
    shop.set_rate(MAIN, "USD", "32.1", "900719.9254").await;
    let mut cmd = sale(
        "large-0002-aaaaaaaaaa",
        &customer,
        &passport,
        "100000000",
        "900719.9254",
    );
    cmd.fee = "100000000".to_owned();
    let saved = shop.save(MAIN, cmd, at(0)).await.expect("the save");

    let stored = found(&shop, &saved.document_ref).await;
    assert_eq!(
        figures(&stored),
        owned("90071992540000.00", "90072092540000.00", "0.00")
    );
    assert_eq!(stored.fee.to_string(), "100000000.00");
    shop.finish().await;
}

#[tokio::test]
async fn one_satang_past_the_input_cap_is_read_back() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "0"), ("USD", "100000000")]).await;
    // a principal of exactly 100,000,000.00 and a received one satang above it
    shop.set_rate(MAIN, "USD", "1", "1").await;
    let mut cmd = sale(
        "large-0003-aaaaaaaaaa",
        &customer,
        &passport,
        "100000000",
        "1",
    );
    cmd.fee = "0.01".to_owned();
    let saved = shop.save(MAIN, cmd, at(0)).await.expect("the save");

    let stored = found(&shop, &saved.document_ref).await;
    assert_eq!(
        figures(&stored),
        owned("100000000.00", "100000000.01", "0.00")
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_large_exchange_sent_again_is_answered_as_the_first() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "0"), ("USD", "100000000")]).await;
    let cmd = sale(
        "large-0004-aaaaaaaaaa",
        &customer,
        &passport,
        "99999999.99",
        "33",
    );
    let first = shop
        .save(MAIN, cmd.clone(), at(0))
        .await
        .expect("the first save");
    let again = shop
        .save(MAIN, cmd, at(30))
        .await
        .expect("the same submission sent again is answered, not refused");
    assert_eq!(again, first);
    assert_eq!(shop.count("transactions").await, 1);
    // the stock moved once
    let balances = shop.balances(MAIN).await.expect("a shift is open");
    assert_eq!(
        (
            balances["THB"].as_str(),
            balances.get("USD").map(String::as_str)
        ),
        ("3299999999.67", Some("0.01"))
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_large_demo_purchase_is_read_back_whole() {
    let shop = Shop::start().await;
    let (_, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let mut who = person();
    who.mode = "DEMO".to_owned();
    let customer = shop.reserve(MAIN, who.clone()).await;
    // a purchase of 99,999,999.99 dollars at 32.1: v2's own example
    let mut cmd = form("large-0005-aaaaaaaaaa", &customer, &passport);
    cmd.customer = who;
    cmd.amount = "99999999.99".to_owned();
    let saved = shop.save(MAIN, cmd, at(0)).await.expect("the DEMO save");
    assert_eq!(saved.mode, Mode::Demo);

    let stored = found(&shop, &saved.document_ref).await;
    assert_eq!(
        figures(&stored),
        owned("3209999999.68", "0.00", "3209999999.68")
    );
    let listed = shop
        .transactions
        .list(&branch(MAIN), Mode::Demo)
        .await
        .expect("list the DEMO exchanges");
    assert_eq!(listed, vec![stored]);
    shop.finish().await;
}

#[tokio::test]
async fn a_stored_figure_the_save_cannot_write_is_still_a_fault() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let saved = shop
        .save(
            MAIN,
            form("large-0006-aaaaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .expect("the save");
    // a negative baht figure: nothing the save writes
    sqlx::query("UPDATE transactions SET principal = -1 WHERE document_ref = $1")
        .bind(&saved.document_ref)
        .execute(&shop.pool)
        .await
        .expect("break the stored row");
    let got = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await;
    match got {
        Err(AppError::Internal(text)) => assert_eq!(text, "stored transaction: principal"),
        other => panic!("expected the fault of the principal column, got {other:?}"),
    }
    shop.finish().await;
}
