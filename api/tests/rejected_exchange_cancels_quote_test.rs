//! A rejected exchange approval cancels the exchange's quote, as a teller's cancel does.
//! Written before the implementation (claude/c4d72c, approved by the BE lane controller claude/7ca80c,
//! 2026-10-15). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/quote-transaction.md`:
//! - `:277-278`: the quote goes `CANCELLED` when "its transaction cancelled before completion";
//! - `:269`: `PENDING_APPROVAL --> CANCELLED : BRANCH_MANAGER rejects (reason)`;
//! - `:368-371`: cancel sets the quote `CANCELLED`, and a `PENDING_APPROVAL` exchange ends only by the
//!   manager's decision, REJECTED ⇒ `CANCELLED` through `on_decided`.
//!
//! On main, `confirm` accepts the quote before it opens the approval request (`confirm.rs` step 4),
//! and `ExchangeTransactionSubject::on_decided` (`approval.rs`) cancels the exchange on REJECTED but
//! leaves its quote `ACCEPTED` and bound to a cancelled exchange. The teller's cancel
//! (`cancel.rs`) already calls `QuoteRepository::cancel_in`, which accepts an `ACCEPTED` quote.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing skips.

mod quote_common;
mod tx4_common;

use quote_common::*;
use tx4_common::*;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionStatus};
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;

const MANAGER: &str = "employee-manager";

/// A MAIN exchange waiting for approval, its quote and its request.
async fn pending(desk: &Desk) -> (Quote, ExchangeTransaction, RequestId) {
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;
    desk.limit("MAIN", "1000.00").await;
    match desk.confirm(t.id, later(now, 10)).await {
        Ok(ConfirmOutcome::PendingApproval(request)) => (q, t, request),
        other => panic!("3210.00 > 1000.00 waits for approval: {other:?}"),
    }
}

#[tokio::test]
async fn a_pending_exchange_holds_its_quote_accepted() {
    let desk = Desk::start().await;
    let (q, t, _) = pending(&desk).await;

    assert_eq!(
        desk.quote_row(&q).await,
        format!("ACCEPTED|{}", t.id.0),
        "confirm accepts the quote before the request opens (§4.1 step 4)"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn a_rejected_exchange_cancels_its_quote() {
    let desk = Desk::start().await;
    let (q, t, request) = pending(&desk).await;

    desk.decide(
        &request,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Rejected,
        "passport photo unclear",
    )
    .await
    .expect("the branch's manager rejects");

    assert_eq!(
        desk.stored(t.id).await.status,
        TransactionStatus::Cancelled,
        "§4.1 reject ⇒ CANCELLED"
    );
    assert_eq!(
        desk.quote_row(&q).await,
        "CANCELLED|",
        "quote-transaction.md:277-278: its transaction was cancelled before completion ⇒ the quote is CANCELLED and bound to nothing"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn an_approved_exchange_keeps_its_quote_accepted() {
    let desk = Desk::start().await;
    let (q, t, request) = pending(&desk).await;

    desk.decide(
        &request,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Approved,
        "known customer",
    )
    .await
    .expect("the branch's manager approves");

    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Confirmed);
    assert_eq!(
        desk.quote_row(&q).await,
        format!("ACCEPTED|{}", t.id.0),
        "approve ⇒ the quote stays ACCEPTED, bound to the confirmed exchange"
    );
    desk.db.finish().await;
}
