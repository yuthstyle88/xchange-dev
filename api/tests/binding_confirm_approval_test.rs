//! Up-front tests for BQ-9 (design `docs/design/blueprint/binding-quote.md` D-20, §4.2 Q33, Q34, Q35): a binding
//! quote whose issue was approved (O-5 A, T2 at issue) is not asked T2 again at confirm. Written by the sub-controller
//! claude/644b3958 on `feat/binding-quote` before the implementation (lane controller's order on PR #5281). **Do not
//! edit, delete or weaken.**
//!
//! Confirm's step 5 (`confirm.rs`) asks T2 when the exchange's settle exceeds the branch limit. It now first asks
//! `binding_approval::covers_in(tx, quote_id, settle)`: the quote is BINDING and names (`quotes.approval_request_id`)
//! an APPROVED `BINDING_QUOTE` request whose `settleThb` is at least the exchange's settle ⇒ CONFIRMED with no
//! `EXCHANGE_TRANSACTION` request. Every other case asks T2 exactly as today.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod binding_issue_common;
mod quote_common;
mod tx4_common;

use binding_issue_common::*;
use quote_common::*;
use tx4_common::*;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_exchange::domain::quote::quote::Quote;
use xc_kernel::money::{Money, Side};
use xchange::application::workflows::exchange::binding_approval::covers_in;
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;

const MANAGER: &str = "employee-manager";

/// A 100 USD binding quote over MAIN's 1000.00 limit, issued under its APPROVED request.
async fn approved_quote(desk: &Desk) -> (Quote, RequestId) {
    desk.limit("MAIN", "1000.00").await;
    let id = RequestId::new(approval_required(
        issue(desk, request(Side::Sell, "100"), whole_second_now()).await,
    ));
    desk.decide(
        &id,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Approved,
        "known customer",
    )
    .await
    .expect("O-5 A: the branch's manager decides");
    let mut retry = request(Side::Sell, "100");
    retry.approval_request_id = Some(id.as_str().to_owned());
    let q = issued(issue(desk, retry, whole_second_now()).await);
    (q.quote, id)
}

async fn exchange_requests(desk: &Desk) -> i64 {
    desk.db
        .count("SELECT count(*) FROM approval_requests WHERE type = 'EXCHANGE_TRANSACTION'")
        .await
}

async fn covers(desk: &Desk, q: &Quote, settle: &str) -> bool {
    let mut tx = desk.db.pool.begin().await.unwrap();
    let got = covers_in(&mut tx, q.id, Money::parse(settle).unwrap())
        .await
        .unwrap_or_else(|e| panic!("covers_in: {e}"));
    tx.rollback().await.unwrap();
    got
}

fn settle(q: &Quote) -> Money {
    let minor: i128 = q.lines.iter().map(|l| l.settle_thb.minor_i128()).sum();
    Money::try_from_minor_i128(minor).unwrap()
}

#[tokio::test]
async fn q33_a_binding_quote_approved_at_issue_confirms_with_no_second_approval() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let (q, _) = approved_quote(&desk).await;
    let now = whole_second_now();
    let t = desk.draft(&q, now).await;

    let got = desk
        .confirm(t.id, now)
        .await
        .unwrap_or_else(|e| panic!("confirm: {e}"));

    assert!(
        matches!(got, ConfirmOutcome::Confirmed),
        "the issue's approval is the T2 decision, got {got:?}"
    );
    assert_eq!(
        exchange_requests(&desk).await,
        0,
        "no EXCHANGE_TRANSACTION request"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q34_a_binding_quote_issued_under_the_limit_still_asks_t2_when_the_limit_drops() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let q = issued(issue(&desk, request(Side::Sell, "100"), whole_second_now()).await).quote;
    desk.limit("MAIN", "1000.00").await;
    let now = whole_second_now();
    let t = desk.draft(&q, now).await;

    let got = desk
        .confirm(t.id, now)
        .await
        .unwrap_or_else(|e| panic!("confirm: {e}"));

    assert!(
        matches!(got, ConfirmOutcome::PendingApproval(_)),
        "no approval at issue ⇒ T2 as today, got {got:?}"
    );
    assert_eq!(exchange_requests(&desk).await, 1);
    desk.db.finish().await;
}

#[tokio::test]
async fn q34_an_indicative_quote_over_the_limit_asks_t2_as_today() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    desk.limit("MAIN", "1000.00").await;
    let now = whole_second_now();
    let (_, t) = desk.main_draft(now).await;

    let got = desk
        .confirm(t.id, now)
        .await
        .unwrap_or_else(|e| panic!("confirm: {e}"));

    assert!(
        matches!(got, ConfirmOutcome::PendingApproval(_)),
        "got {got:?}"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q35_covers_in_needs_an_approved_binding_quote_request_of_at_least_the_amount() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let (q, id) = approved_quote(&desk).await;
    let amount = settle(&q);
    let approved = Money::try_from_minor_i128(amount.minor_i128())
        .unwrap()
        .to_string();
    let one_more = Money::try_from_minor_i128(amount.minor_i128() + 1)
        .unwrap()
        .to_string();

    assert!(covers(&desk, &q, &approved).await, "the approved amount");
    assert!(
        !covers(&desk, &q, &one_more).await,
        "more than was approved"
    );

    desk.db
        .exec(&format!(
            "UPDATE approval_requests SET status = 'REJECTED' WHERE id = '{}'",
            id.as_str()
        ))
        .await;
    assert!(!covers(&desk, &q, &approved).await, "not APPROVED");
    desk.db.finish().await;
}

#[tokio::test]
async fn q35_covers_in_is_false_for_a_quote_with_no_request() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let q = issued(issue(&desk, request(Side::Sell, "10"), whole_second_now()).await).quote;
    assert!(!covers(&desk, &q, "0.01").await);
    let indicative = desk.main_draft(whole_second_now()).await.0;
    assert!(!covers(&desk, &indicative, "0.01").await);
    desk.db.finish().await;
}
