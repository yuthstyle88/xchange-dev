//! Rate feed F11 — a desk save on a tiered publication is checked against the tier of its amount
//! (`docs/design/rate-feed-connector.md` §9.4 last row; owner's ruling R-4 on #986).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! What this pins: `save_transaction.rs` checks the desk's per-1 rate against
//! `PublishedRates::line_for` with the saved foreign amount (group `ALL`), not `line_in`. On a
//! tiered publication the rate of the amount's tier is accepted and that line is stored; another
//! tier's rate is refused `RATE_MISMATCH`. An untiered publication is checked as before (the
//! existing `rates_re4_desk_save_test` suite).

mod save_common;

use rust_decimal::Decimal;
use save_common::rates_re4c_common::{d, publish};
use save_common::*;

/// A cash exchange of `amount` `currency` at the per-1 `rate` the desk sends, LIVE, fee 0.
fn exchange(
    submission: &str,
    customer: &str,
    passport: &str,
    side: &str,
    currency: &str,
    amount: &str,
    rate: &str,
) -> SaveTransaction {
    let mut cmd = form(submission, customer, passport);
    cmd.side = side.to_owned();
    cmd.currency = currency.to_owned();
    cmd.amount = amount.to_owned();
    cmd.rate = rate.to_owned();
    cmd
}

/// `MAIN` with an open shift, a customer and a passport file. Answers (customer, passport).
async fn counter(shop: &Shop) -> (String, String) {
    shop.open_shift(MAIN, &[("THB", "1000000"), ("USD", "10000")])
        .await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    (customer, passport)
}

/// Publishes USD on MAIN, then replaces its `'ALL'` line with three tiers (base BUY 32.00).
/// Answers the tier line ids, T1 first.
async fn publish_tiered_usd(shop: &Shop) -> Vec<String> {
    let p = publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    let mut ids = Vec::new();
    for (tier, lo, hi, buy, sell) in [
        ("T1", None, Some("10000.00"), "31.50", "33.00"),
        ("T2", Some("10000.00"), Some("50000.00"), "31.70", "32.80"),
        ("T3", Some("50000.00"), None, "31.90", "32.60"),
    ] {
        let id: String = sqlx::query_scalar(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                 denomination_group, unit_size, buy_rate, sell_rate, source_snapshot, value_tier, \
                 tier_lower_exclusive_thb, tier_upper_inclusive_thb, base_buy_rate, base_sell_rate) \
             SELECT gen_random_uuid(), organization_id, publication_id, currency_code, 'ALL', 1, \
                 $2::numeric, $3::numeric, source_snapshot, $4, $5::numeric, $6::numeric, 32.00, 32.50 \
             FROM published_rate_lines WHERE publication_id = $1::uuid AND currency_code = 'USD' \
               AND value_tier = 'ALL' \
             RETURNING id::text",
        )
        .bind(&p.id)
        .bind(buy)
        .bind(sell)
        .bind(tier)
        .bind(lo)
        .bind(hi)
        .fetch_one(&shop.pool)
        .await
        .expect("a USD tier line");
        ids.push(id);
    }
    sqlx::query(
        "DELETE FROM published_rate_lines WHERE publication_id = $1::uuid AND currency_code = 'USD' \
           AND value_tier = 'ALL'",
    )
    .bind(&p.id)
    .execute(&shop.pool)
    .await
    .expect("drop the untiered USD line");
    ids
}

async fn stored(shop: &Shop, document_ref: &str) -> (Option<String>, Decimal) {
    sqlx::query_as(
        // TX-9 #677: the published line through the first line's quote line, the rate off the line.
        "SELECT ql.published_rate_line_id::text, l.rate_snapshot \
         FROM transaction_lines l JOIN transactions t ON t.id = l.transaction_id \
         LEFT JOIN quote_lines ql ON ql.id = l.quote_line_id \
         WHERE t.document_ref = $1 AND l.line_no = 1",
    )
    .bind(document_ref)
    .fetch_one(&shop.pool)
    .await
    .unwrap_or_else(|e| panic!("transaction {document_ref}: {e}"))
}

#[tokio::test]
async fn a_save_at_the_rate_of_its_tier_is_accepted_and_stores_that_line() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop).await;
    let ids = publish_tiered_usd(&shop).await;

    // 500 USD × base 32.00 = 16,000.00 THB → T2, bought at 31.70.
    let saved = shop
        .save(
            MAIN,
            exchange(
                "f11-tier-buy-000001",
                &customer,
                &passport,
                "BUY",
                "USD",
                "500",
                "31.7",
            ),
            at(0),
        )
        .await
        .expect("the save at the T2 rate is accepted");
    assert_eq!(
        stored(&shop, &saved.document_ref).await,
        (Some(ids[1].clone()), d("31.7")),
        "the T2 line and its rate are stored"
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_save_at_another_tiers_rate_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop).await;
    publish_tiered_usd(&shop).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "f11-tier-buy-000002",
                &customer,
                &passport,
                "BUY",
                "USD",
                "500",
                "31.5",
            ),
            at(0),
        )
        .await;
    assert_eq!(
        refusal(refused),
        RATE_MISMATCH,
        "31.50 is T1's rate; 500 USD is in T2"
    );
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    shop.finish().await;
}
