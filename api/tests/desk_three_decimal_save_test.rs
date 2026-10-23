//! #4996 (Q4 of #4961, three-decimal currencies): the desk save takes the foreign amount by the currency's minor
//! unit, so `KWD 1.125` is saved as itself, never refused and never rounded.
//!
//! Written before the implementation by the ticket author's lane (claude/be38637e, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Design `docs/design/three-decimal-currencies.md` §4:
//! - K1. desk save, BUY `KWD 1.125` → saved; `transaction_lines.quantity = 1.1250`; till stock `+1.125`.
//! - K3. desk save `KWD 1.1255` → refused `TooManyDecimals` (`TOO_MANY_DECIMALS`), nothing stored.
//! - K5. `USD 10.50` / `JPY 1500` / `JPY 1.5` → accepted / accepted / refused, as today.
//!
//! Today K1 is refused: `TransactionForm::parse` reads the amount with `Money::parse`, 2 places
//! (`transaction_form.rs:130`). K3 and K5 hold today and must still hold after Q4.
//!
//! The save is the desk's `saveTransaction` (`POST /api/staff/rpc` → `SaveTransactionWorkflow::save`), driven through
//! `save_common`'s shop: a LIVE cash exchange of the owner on `MAIN`'s open shift, priced at the current publication
//! (`rates_re4c_common::publish`). The desk sends the rate per 1; KWD and USD are priced per 1, JPY per 100. The
//! command is text, so this file compiles on today's types and on Q4's.
//!
//! Needs `TEST_DATABASE_URL` (see `save_common/mod.rs`); nothing here skips.

mod save_common;

use rust_decimal::Decimal;
use save_common::rates_re4c_common::{d, publish};
use save_common::*;

const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";

/// A LIVE cash exchange of `amount` `currency` on `side`, at the per-1 `rate` the desk sends, fee 0.
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

/// `MAIN` with an open shift holding THB 100,000, the customer of `person()`, a passport file of the owner, and a
/// publication of KWD (100 / 101 per 1), USD (35 / 36 per 1) and JPY (23 / 24 per 100). Answers (customer, passport).
async fn counter(shop: &Shop) -> (String, String) {
    shop.open_shift(MAIN, &[("THB", "100000")]).await;
    publish(
        &shop.pool,
        MAIN,
        &[
            ("KWD", "100", "101"),
            ("USD", "35", "36"),
            ("JPY", "23", "24"),
        ],
    )
    .await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    (customer, passport)
}

/// The first line of the saved exchange: (currency, quantity, THB amount).
async fn line_of(shop: &Shop, document_ref: &str) -> (String, Decimal, Decimal) {
    sqlx::query_as(
        "SELECT l.currency::text, l.quantity, l.thb_amount \
         FROM transaction_lines l JOIN transactions t ON t.id = l.transaction_id \
         WHERE t.document_ref = $1 AND l.line_no = 1",
    )
    .bind(document_ref)
    .fetch_one(&shop.pool)
    .await
    .unwrap_or_else(|e| panic!("the line of {document_ref}: {e}"))
}

/// The open shift's expected balance of `code`, as a number (absent = 0).
async fn till(shop: &Shop, code: &str) -> Decimal {
    let balances = shop.balances(MAIN).await.expect("the shift is open");
    balances
        .get(code)
        .map(|text| {
            text.parse::<Decimal>()
                .unwrap_or_else(|e| panic!("{code} balance {text:?}: {e}"))
        })
        .unwrap_or(Decimal::ZERO)
}

// ------------------------------------------------------------ the rows

/// K1: BUY KWD 1.125 at 100 per 1 is saved as 1.125 (stored 1.1250), pays THB 112.50, and the till gains KWD 1.125.
#[tokio::test]
async fn k1_a_kwd_buy_of_1_125_is_saved_with_three_places() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop).await;

    let saved = shop
        .save(
            MAIN,
            exchange(
                "k1-kwd-buy-1125",
                &customer,
                &passport,
                "BUY",
                "KWD",
                "1.125",
                "100",
            ),
            at(0),
        )
        .await
        .expect("K1: the desk save of BUY KWD 1.125 (KWD has 3 decimals) is accepted");

    let (currency, quantity, thb) = line_of(&shop, &saved.document_ref).await;
    assert_eq!(currency, "KWD");
    assert_eq!(
        quantity,
        d("1.1250"),
        "K1: transaction_lines.quantity = 1.1250"
    );
    assert_eq!(thb, d("112.50"), "K1: 1.125 × 100 = THB 112.50");
    assert_eq!(
        till(&shop, "KWD").await,
        d("1.125"),
        "K1: the till holds KWD 1.125"
    );
    assert_eq!(
        till(&shop, "THB").await,
        d("99887.50"),
        "K1: the till paid THB 112.50"
    );
    shop.finish().await;
}

/// K3: KWD 1.1255 has four places, one more than KWD's minor unit: refused `TOO_MANY_DECIMALS`, nothing written.
#[tokio::test]
async fn k3_a_kwd_amount_with_four_places_is_refused_and_nothing_is_stored() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "k3-kwd-buy-11255",
                &customer,
                &passport,
                "BUY",
                "KWD",
                "1.1255",
                "100",
            ),
            at(0),
        )
        .await;
    assert_eq!(
        refusal(refused),
        TOO_MANY_DECIMALS,
        "K3: KWD 1.1255 at the desk save"
    );
    assert_eq!(
        shop.everything().await,
        before,
        "K3: the refused save wrote nothing"
    );
    assert_eq!(till(&shop, "KWD").await, Decimal::ZERO);
    shop.finish().await;
}

/// K5: USD 10.50 (2 places) is accepted as today.
#[tokio::test]
async fn k5_a_usd_amount_with_two_places_is_accepted_as_today() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop).await;

    let saved = shop
        .save(
            MAIN,
            exchange(
                "k5-usd-buy-1050",
                &customer,
                &passport,
                "BUY",
                "USD",
                "10.50",
                "35",
            ),
            at(0),
        )
        .await
        .expect("K5: the desk save of BUY USD 10.50 is accepted");
    let (currency, quantity, thb) = line_of(&shop, &saved.document_ref).await;
    assert_eq!(
        (currency.as_str(), quantity, thb),
        ("USD", d("10.50"), d("367.50")),
        "K5: USD 10.50 × 35"
    );
    assert_eq!(till(&shop, "USD").await, d("10.50"));
    shop.finish().await;
}

/// K5: JPY 1500 (no places) is accepted as today; JPY is priced per 100, so the desk's per-1 rate is 0.23.
#[tokio::test]
async fn k5_a_whole_jpy_amount_is_accepted_as_today() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop).await;

    let saved = shop
        .save(
            MAIN,
            exchange(
                "k5-jpy-buy-1500",
                &customer,
                &passport,
                "BUY",
                "JPY",
                "1500",
                "0.23",
            ),
            at(0),
        )
        .await
        .expect("K5: the desk save of BUY JPY 1500 is accepted");
    let (currency, quantity, thb) = line_of(&shop, &saved.document_ref).await;
    assert_eq!(
        (currency.as_str(), quantity, thb),
        ("JPY", d("1500"), d("345.00")),
        "K5: JPY 1500 × 23 ÷ 100"
    );
    assert_eq!(till(&shop, "JPY").await, d("1500"));
    shop.finish().await;
}

/// K5: JPY 1.5 has a place JPY does not have: refused as today, nothing written.
#[tokio::test]
async fn k5_a_jpy_amount_with_a_place_is_refused_as_today() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "k5-jpy-buy-15",
                &customer,
                &passport,
                "BUY",
                "JPY",
                "1.5",
                "0.23",
            ),
            at(0),
        )
        .await;
    assert!(
        refused.is_err(),
        "K5: the desk save of JPY 1.5 is refused; got {refused:?}"
    );
    assert_eq!(
        shop.everything().await,
        before,
        "K5: the refused JPY 1.5 save wrote nothing"
    );
    shop.finish().await;
}
