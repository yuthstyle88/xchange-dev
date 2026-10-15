//! TX-9 #677 prerequisite — a save no longer writes the wide `transactions.published_rate_line_id`. The
//! published line a save was priced on is read through `transaction_lines.quote_line_id → quote_lines`
//! (BE controller's ruling, #677 issuecomment-6183687076). Written before the implementation; do not edit,
//! delete or weaken.
//!
//! 1. `a_desk_save_links_its_published_line_through_the_quote_line`: green on main, a pin.
//! 2. `a_save_needs_no_wide_published_line_column`: red on main, because the `UPDATE transactions SET
//!    published_rate_line_id` in `api/src/application/workflows/save_transaction.rs` fails on a dropped column.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod save_common;

use save_common::rates_re4c_common::publish;
use save_common::*;

/// `MAIN` with an open shift, a customer and a passport file, and USD published at BUY 32.1 / SELL 33.
/// Answers (customer, passport, the published USD line's id).
async fn priced_counter(shop: &Shop) -> (String, String, String) {
    shop.open_shift(MAIN, &[("THB", "1000000"), ("USD", "10000")])
        .await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    let p = publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    let line: String = sqlx::query_scalar(
        "SELECT id::text FROM published_rate_lines \
         WHERE publication_id = $1::uuid AND currency_code = 'USD'",
    )
    .bind(&p.id)
    .fetch_one(&shop.pool)
    .await
    .expect("the published USD line");
    (customer, passport, line)
}

/// The published line of a saved exchange's first line, read through its quote line.
async fn linked_line(shop: &Shop, document_ref: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT ql.published_rate_line_id::text FROM transactions t \
         JOIN transaction_lines l ON l.transaction_id = t.id AND l.line_no = 1 \
         JOIN quote_lines ql ON ql.id = l.quote_line_id \
         WHERE t.document_ref = $1 AND t.branch_id = $2",
    )
    .bind(document_ref)
    .bind(MAIN)
    .fetch_optional(&shop.pool)
    .await
    .expect("read the linked published line")
}

// 1.
#[tokio::test]
async fn a_desk_save_links_its_published_line_through_the_quote_line() {
    let shop = Shop::start().await;
    let (customer, passport, line) = priced_counter(&shop).await;

    let saved = shop
        .save(
            MAIN,
            form("tx9-pub-0001-aaaaaaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .expect("a desk save at the published rate");

    assert_eq!(
        linked_line(&shop, &saved.document_ref).await.as_deref(),
        Some(line.as_str()),
        "the line's quote line names the published line the save was priced on"
    );

    shop.finish().await;
}

// 2.
#[tokio::test]
async fn a_save_needs_no_wide_published_line_column() {
    let shop = Shop::start().await;
    let (customer, passport, line) = priced_counter(&shop).await;
    // TX-9's migration 3 drops the column. `IF EXISTS` keeps this test green after it does.
    sqlx::query("ALTER TABLE transactions DROP COLUMN IF EXISTS published_rate_line_id")
        .execute(&shop.pool)
        .await
        .expect("drop the wide column in this test's schema");

    let saved = shop
        .save(
            MAIN,
            form("tx9-pub-0002-aaaaaaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .expect("a save writes nothing to transactions.published_rate_line_id");

    assert_eq!(
        linked_line(&shop, &saved.document_ref).await.as_deref(),
        Some(line.as_str()),
        "the link of record is still written"
    );

    shop.finish().await;
}
