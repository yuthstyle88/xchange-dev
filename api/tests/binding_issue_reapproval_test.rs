//! Up-front tests for BQ-10 (design `docs/design/blueprint/binding-quote.md` D-15, D-20, §4.2 Q36, Q37): an
//! APPROVED `BINDING_QUOTE` request is approval for its `settleThb` and no more, and a proposal can be asked
//! again once its earlier request is decided or used. Written by the sub-controller claude/studio-xc-bq on
//! `feat/binding-quote` before the implementation (claude/2ca5874b's FAIL on PR #5281, must-fix 1 and 2).
//! **Do not edit, delete or weaken.**
//!
//! MAIN sells USD at 32.45 (`Desk::start`), so 100 USD settles 3245.00, over MAIN's 1000.00 limit. A new
//! publication moves the price without changing the proposal (`proposal_id` hashes no price).
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod binding_issue_common;
mod quote_common;
mod tx4_common;

use binding_issue_common::*;
use quote_common::*;
use tx4_common::*;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::binding_issue::{
    APPROVAL_PRICE_MOVED, APPROVAL_REJECTED, BindingIssueRequest,
};

const MANAGER: &str = "employee-manager";

/// MAIN sells USD at `sell` from now on.
async fn sell_usd_at(desk: &Desk, sell: &'static str) {
    desk.db
        .publish("MAIN", &[Line::manual("USD", "1", "32.10", sell)])
        .await;
}

/// The first try of 100 USD at MAIN, over the 1000.00 limit: its PENDING request.
async fn pending(desk: &Desk) -> RequestId {
    desk.limit("MAIN", "1000.00").await;
    RequestId::new(approval_required(
        issue(desk, request(Side::Sell, "100"), whole_second_now()).await,
    ))
}

/// `pending`, then decided by MAIN's manager.
async fn decided(desk: &Desk, outcome: Outcome) -> RequestId {
    let id = pending(desk).await;
    desk.decide(
        &id,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        outcome,
        "known customer",
    )
    .await
    .expect("O-5 A: the branch's manager decides");
    id
}

fn naming(id: &RequestId) -> BindingIssueRequest {
    let mut r = request(Side::Sell, "100");
    r.approval_request_id = Some(id.as_str().to_owned());
    r
}

async fn settle_of(desk: &Desk, id: &RequestId) -> String {
    desk.db
        .text(&format!(
            "SELECT proposed_payload->>'settleThb' FROM approval_requests WHERE id = '{}'",
            id.as_str()
        ))
        .await
        .expect("the request is a row")
}

#[tokio::test]
async fn q36_an_approved_request_does_not_issue_above_its_approved_amount() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = decided(&desk, Outcome::Approved).await;
    assert_eq!(settle_of(&desk, &id).await, "3245.00");
    sell_usd_at(&desk, "33.00").await;
    let (quotes_before, reservations_before) = (quotes(&desk).await, reservations(&desk).await);

    let got = issue(&desk, naming(&id), whole_second_now()).await;

    assert_eq!(
        refusal(got),
        APPROVAL_PRICE_MOVED,
        "3300.00 > the approved 3245.00"
    );
    assert_eq!(quotes(&desk).await, quotes_before, "no quote");
    assert_eq!(
        reservations(&desk).await,
        reservations_before,
        "no reservation"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q36_an_approved_request_still_issues_once_the_price_is_back_within_it() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = decided(&desk, Outcome::Approved).await;
    sell_usd_at(&desk, "33.00").await;
    let _ = refusal(issue(&desk, naming(&id), whole_second_now()).await);
    sell_usd_at(&desk, "32.20").await;

    let q = issued(issue(&desk, naming(&id), whole_second_now()).await);

    let total: String = desk
        .db
        .text(&format!(
            "SELECT sum(settle_thb)::numeric(20,2)::text FROM quote_lines WHERE quote_id = '{}'",
            q.quote.id
        ))
        .await
        .expect("the quote has lines");
    assert_eq!(total, "3220.00", "at or below the approved amount ⇒ issued");
    desk.db.finish().await;
}

#[tokio::test]
async fn q37_a_used_proposal_is_asked_again_after_its_quote_is_cancelled() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = decided(&desk, Outcome::Approved).await;
    let first = issued(issue(&desk, naming(&id), whole_second_now()).await);
    desk.db
        .exec(&format!(
            "UPDATE quotes SET status = 'CANCELLED' WHERE id = '{}'",
            first.quote.id
        ))
        .await;

    let again =
        approval_required(issue(&desk, request(Side::Sell, "100"), whole_second_now()).await);

    assert_ne!(again, id.as_str(), "a new request, not the used one");
    let rows = binding_requests(&desk).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows[1].ends_with("|PENDING"), "{rows:?}");
    desk.db.finish().await;
}

#[tokio::test]
async fn q37_an_approved_unused_proposal_whose_price_rose_is_asked_again() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = decided(&desk, Outcome::Approved).await;
    sell_usd_at(&desk, "33.00").await;

    let again =
        approval_required(issue(&desk, request(Side::Sell, "100"), whole_second_now()).await);

    assert_ne!(again, id.as_str(), "a new request for the new amount");
    assert_eq!(settle_of(&desk, &RequestId::new(again)).await, "3300.00");
    desk.db.finish().await;
}

#[tokio::test]
async fn q37_a_rejected_proposal_whose_price_moved_is_asked_again() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = decided(&desk, Outcome::Rejected).await;
    sell_usd_at(&desk, "32.20").await;

    let again =
        approval_required(issue(&desk, request(Side::Sell, "100"), whole_second_now()).await);

    assert_ne!(again, id.as_str());
    assert_eq!(settle_of(&desk, &RequestId::new(again)).await, "3220.00");
    desk.db.finish().await;
}

#[tokio::test]
async fn q37_a_rejected_proposal_at_the_same_amount_stays_rejected() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let _ = decided(&desk, Outcome::Rejected).await;

    let got = issue(&desk, request(Side::Sell, "100"), whole_second_now()).await;

    assert_eq!(refusal(got), APPROVAL_REJECTED);
    assert_eq!(binding_requests(&desk).await.len(), 1, "no new request");
    desk.db.finish().await;
}

#[tokio::test]
async fn q37_a_pending_proposal_whose_price_moved_answers_the_pending_request() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = pending(&desk).await;
    sell_usd_at(&desk, "33.00").await;

    let got = approval_required(issue(&desk, request(Side::Sell, "100"), whole_second_now()).await);

    assert_eq!(
        got,
        id.as_str(),
        "one PENDING request per proposal, not a 500"
    );
    assert_eq!(binding_requests(&desk).await.len(), 1);
    desk.db.finish().await;
}
