//! Ticket #147 — a rate text that is not a plain decimal. Written before the fix. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `KioskService::configure` judges the two texts of a counter rate with a general decimal
//! parser, so `1_0` is read as 10, `+1` as 1, `.5` as 0.5 and `5.` as 5, and the text is then
//! handed on to the rate save and written to the audit line as it came (ticket #129). The rule is
//! "a plain decimal": ASCII digits with at most one dot that has a digit on each side
//! (`xc_kernel::money::plain_decimal`). A text that is not one is refused with the rates-first
//! sentence, before anything is saved or written.
//!
//! The values are chosen so that the other comparison of `configure` (`buy > 0`, `sell >= buy`)
//! passes when the odd text is read leniently: the refusal can only come from the rule.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they FAIL; none skips.
//!
//! Amended by the ticket author of #603 (RE-6, claude/e5ab6c): `configure` no longer reads,
//! saves or judges the rate texts (item 1); it switches a currency on only when the branch's
//! current publication has it on both sides (item 2). The two odd-text tests still hold (nothing
//! is published, so the rates-first refusal comes and nothing is written).
//! `plain_texts_are_saved_as_they_came` publishes CHF first and pins that no rate is saved.

mod kiosk_service_common;

use kiosk_service_common::{
    FakePorts, MAIN, TestDb, at, branch, configure_of, exact, owner, publish_one, refusal,
};

const RATES_FIRST: &str = "Set valid counter buy and sell rates first.";

/// `(buy, sell)` pairs in which `buy` is the odd text and `sell` a plain one above it.
const ODD_BUY: [(&str, &str); 4] = [
    ("1_0", "37.1"),
    ("+1", "37.1"),
    (".5", "37.1"),
    ("5.", "37.1"),
];

/// `(buy, sell)` pairs in which `sell` is the odd text and `buy` a plain one it is not below.
const ODD_SELL: [(&str, &str); 4] = [("5", "1_0"), ("1", "+1"), ("0.5", ".5"), ("5", "5.")];

async fn nothing_is_saved_for(cases: &[(&str, &str)]) {
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    for (buy, sell) in cases {
        let sentence = refusal(
            service
                .configure(
                    &branch(MAIN),
                    &owner(),
                    configure_of("CHF", true, buy, sell),
                    at(1_000),
                )
                .await,
        );
        assert_eq!(
            sentence, RATES_FIRST,
            "buy {buy:?} / sell {sell:?}: not a plain decimal"
        );
        assert!(
            ports.rate_saves().is_empty(),
            "buy {buy:?} / sell {sell:?}: a rate was saved from a text that is not a plain decimal"
        );
        assert_eq!(
            db.count("test_audit").await,
            0,
            "buy {buy:?} / sell {sell:?}: an audit line was written"
        );
        assert_eq!(
            db.count("kiosk_currencies").await,
            0,
            "buy {buy:?} / sell {sell:?}: a switch was stored"
        );
    }
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_buy_that_is_not_a_plain_decimal_saves_and_writes_nothing() {
    nothing_is_saved_for(&ODD_BUY).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sell_that_is_not_a_plain_decimal_saves_and_writes_nothing() {
    nothing_is_saved_for(&ODD_SELL).await;
}

/// Holds before the fix and after it: one audit line, one switch. Amended for #603: CHF is
/// published first, and no rate save is asked for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plain_texts_are_saved_as_they_came() {
    let db = TestDb::start().await;
    publish_one(&db.pool, MAIN, exact(0), "CHF", "36.4", "37.1").await;
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    service
        .configure(
            &branch(MAIN),
            &owner(),
            configure_of("CHF", true, "36.4", "37.1"),
            at(1_000),
        )
        .await
        .expect("a plain buy and sell are accepted");
    assert!(
        ports.rate_saves().is_empty(),
        "#603 item 1: configure asks for no rate save"
    );
    assert_eq!(db.count("test_audit").await, 1);
    assert_eq!(db.count("kiosk_currencies").await, 1);
    db.finish().await;
}
