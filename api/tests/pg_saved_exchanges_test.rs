//! Written before the implementation (ticket T13b-S). Do not edit, delete or weaken.
//!
//! `PgSavedExchanges` — what the device service is told about a saved exchange,
//! answered from `TransactionService::find` (`docs/design/xchange-v3.md` §5.4):
//!
//! - a live exchange of the branch with that document number → `Completed`.
//!   v3 stores no exchange that is pending or cancelled, so `NotCompleted` is
//!   never answered;
//! - no exchange of the branch with that number → `None`. The number is
//!   compared exactly (`saved_exchanges.rs`), and another branch's exchange is
//!   not this branch's;
//! - a `DEMO` exchange → `None`: only live exchanges count;
//! - a database failure → the error, never "not found".
//!
//! Needs `TEST_DATABASE_URL` (see `save_common/mod.rs`); nothing here skips.

mod common;
mod save_common;

use std::sync::Arc;

use pos_infra::transactions::Transactor;
use save_common::*;
use xc_kernel::branch_tx::BranchTx;
use xchange::infrastructure::devices::pg_saved_exchanges::PgSavedExchanges;
use xchange::infrastructure::devices::saved_exchanges::{ExchangeState, SavedExchanges};

/// A shop with one live and one DEMO exchange saved in `MAIN`. Answers the
/// shop and the two document numbers (live, demo).
async fn a_shop_with_two_exchanges() -> (Shop, String, String) {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let live = shop
        .save(
            MAIN,
            form("live-0001-aaaaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .expect("the live save");
    assert_eq!(live.mode, Mode::Live);

    let mut demo_person = person();
    demo_person.mode = "DEMO".to_owned();
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;
    let mut demo = form("demo-0001-aaaaaaaaaa", &demo_customer, &passport);
    demo.customer = demo_person;
    let demo = shop.save(MAIN, demo, at(60)).await.expect("the DEMO save");
    assert_eq!(demo.mode, Mode::Demo);
    assert_ne!(live.document_ref, demo.document_ref);
    (shop, live.document_ref, demo.document_ref)
}

fn exchanges(shop: &Shop) -> PgSavedExchanges {
    PgSavedExchanges::new(shop.transactions.clone())
}

async fn find(of: &PgSavedExchanges, branch_id: &str, reference: &str) -> Option<ExchangeState> {
    of.find(&branch(branch_id), reference)
        .await
        .unwrap_or_else(|e| panic!("find {branch_id}/{reference}: {e:?}"))
}

#[tokio::test]
async fn a_live_exchange_of_the_branch_is_completed() {
    let (shop, live, _) = a_shop_with_two_exchanges().await;
    assert_eq!(
        find(&exchanges(&shop), MAIN, &live).await,
        Some(ExchangeState::Completed)
    );
}

#[tokio::test]
async fn a_number_the_branch_does_not_have_is_none() {
    let (shop, live, _) = a_shop_with_two_exchanges().await;
    let of = exchanges(&shop);
    assert_eq!(find(&of, MAIN, "XC-20990101-000001").await, None);
    assert_eq!(find(&of, MAIN, "").await, None);
    // compared exactly: no trimming, no change of case, no prefix match
    assert_eq!(find(&of, MAIN, &format!(" {live}")).await, None);
    assert_eq!(find(&of, MAIN, &live.to_lowercase()).await, None);
    assert_eq!(find(&of, MAIN, &live[..live.len() - 1]).await, None);
    // the live one is still found, so the misses above are not a broken look-up
    assert_eq!(find(&of, MAIN, &live).await, Some(ExchangeState::Completed));
}

#[tokio::test]
async fn another_branchs_exchange_is_not_this_branchs() {
    let (shop, live, _) = a_shop_with_two_exchanges().await;
    shop.seed_branch("XC-B02", "Second").await;
    let of = exchanges(&shop);
    assert_eq!(find(&of, "XC-B02", &live).await, None);
    assert_eq!(find(&of, MAIN, &live).await, Some(ExchangeState::Completed));
}

#[tokio::test]
async fn a_demo_exchange_is_none() {
    let (shop, live, demo) = a_shop_with_two_exchanges().await;
    let of = exchanges(&shop);
    assert_eq!(find(&of, MAIN, &demo).await, None);
    assert_eq!(find(&of, MAIN, &live).await, Some(ExchangeState::Completed));
}

#[tokio::test]
async fn a_database_failure_is_an_error_not_a_miss() {
    let pool = common::unreachable_pool();
    let transactions = TransactionService::new(
        Arc::new(PgTransactionRepository::new(pool.clone())),
        BranchTx::new(Transactor::new(pool)),
    );
    let of = PgSavedExchanges::new(transactions);
    let answer = of.find(&branch(MAIN), "XC-20261006-000001").await;
    assert!(
        answer.is_err(),
        "a database failure must not read as {answer:?}"
    );
}
