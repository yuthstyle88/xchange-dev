//! Ticket #672 (TX-4) — `ConfirmTransaction::run_in` and `CancelTransaction::run_in` against a
//! real PostgreSQL: the branch limit decides CONFIRMED or PENDING_APPROVAL, the quote is checked
//! and accepted, every registered `SaveStep::before_in` runs, and cancel's three starting states.
//! Written before the implementation (claude/14cc24bd, on the owner's order, 2026-10-10). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §0 T2 (above a per-branch THB limit,
//! BRANCH_MANAGER approves; no limit ⇒ nothing waits), §2 D-5 (`Quote::is_valid(now, current)` =
//! `now < expires_at` and the publication is the branch's current one), D-6 (checked at confirm),
//! D-7 (the steps run unchanged), §4.1 (the states), §4.4 `confirm` steps 1–5, §5.2 (the limit
//! compares `settle_thb`, `[proposed]`, owner question Q-3), §7 (signatures). Ticket #672 G1
//! (`accept_in`, `cancel_in`), G3 (the requester `Actor` is built from the **transaction's**
//! branch; the request key is `confirm:<transaction id>`), §9 (edge cases).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. The DRAFTs are DEMO and stored through #670's store (`tx4_common`). Every call is
//! committed whatever it answers, and the rows are read back.
//!
//! Signatures this test assumes ([required] in ticket §7 unless marked "choice"):
//! - `xchange::application::workflows::exchange::confirm::{ConfirmTransaction, ConfirmOutcome}`:
//!   - choice: `ConfirmTransaction::new(pool: PgPool, approvals: Arc<ApprovalService>, steps:
//!     Vec<Arc<dyn SaveStep>>)`;
//!   - `run_in(&self, tx, id: TransactionId, who: &SaveActor, extras: &SaveExtras, now) ->
//!     AppResult<ConfirmOutcome>` (async);
//!   - `ConfirmOutcome::{Confirmed, PendingApproval(RequestId)}`, `Debug` + `PartialEq`, where
//!     `RequestId` is AP-1's `xc_approval::domain::request::RequestId` (the design's
//!     `ApprovalRequestId`).
//! - `xchange::application::workflows::exchange::cancel::CancelTransaction`:
//!   - choice: `CancelTransaction::new(pool: PgPool)`;
//!   - `run_in(&self, tx, id, reason: &str, who: &SaveActor, now) ->
//!     AppResult<ExchangeTransaction>` (async), answering the cancelled exchange.
//! - Refusal codes, as `Err(AppError::Domain(m))` with `m` starting with the code:
//!   `staleVersion` (not the state the move starts from), `quoteExpired` (D-5 fails at confirm),
//!   `validationFailed` (choice: an empty or blank cancel reason).
//! - A step's own refusal is answered unchanged (D-7: the steps are reused as they are).

mod quote_common;
mod tx4_common;

use quote_common::*;
use tx4_common::*;
use xc_exchange::domain::transaction::transaction::TransactionStatus;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;
use xchange::application::workflows::save_step::SaveExtras;

// ---- confirm: the limit ----

#[tokio::test]
async fn with_no_limit_set_confirm_confirms_and_the_quote_is_accepted_by_the_exchange() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;

    assert_eq!(
        desk.confirm(t.id, later(now, 10)).await.expect("confirmed"),
        ConfirmOutcome::Confirmed,
        "T2: no limit set ⇒ never pending"
    );
    let stored = desk.stored(t.id).await;
    assert_eq!(stored.status, TransactionStatus::Confirmed);
    assert_eq!(stored.approval_request_id, None);
    assert_eq!(
        stored.document_ref, None,
        "D-3: numbered only at completion"
    );
    assert_eq!(
        desk.quote_row(&q).await,
        format!("ACCEPTED|{}", t.id.0),
        "§4.4 step 4: the quote is accepted, bound to this exchange"
    );
    assert_eq!(desk.requests().await, 0);
    desk.db.finish().await;
}

#[tokio::test]
async fn a_settle_amount_at_the_limit_confirms_and_one_satang_over_waits() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (_, at_limit) = desk.main_draft(now).await;
    let (_, over) = desk.main_draft(now).await;

    desk.limit("MAIN", "3210.00").await;
    assert_eq!(
        desk.confirm(at_limit.id, later(now, 10))
            .await
            .expect("confirmed"),
        ConfirmOutcome::Confirmed,
        "settle 3210.00 is not above a limit of 3210.00"
    );

    desk.limit("MAIN", "3209.99").await;
    let outcome = desk
        .confirm(over.id, later(now, 10))
        .await
        .expect("an outcome");
    assert!(
        matches!(outcome, ConfirmOutcome::PendingApproval(_)),
        "settle 3210.00 > 3209.99 ⇒ PENDING_APPROVAL, as an outcome, not an error: {outcome:?}"
    );
    assert_eq!(
        desk.stored(over.id).await.status,
        TransactionStatus::PendingApproval
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn over_the_limit_one_request_is_opened_for_the_exchange_at_its_branch_by_the_teller() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;
    desk.limit("MAIN", "1000.00").await;

    let ConfirmOutcome::PendingApproval(request) = desk
        .confirm(t.id, later(now, 10))
        .await
        .expect("an outcome")
    else {
        panic!("3210.00 > 1000.00 waits for approval");
    };

    let stored = desk.stored(t.id).await;
    assert_eq!(stored.status, TransactionStatus::PendingApproval);
    assert_eq!(
        stored.approval_request_id.map(|u| u.to_string()),
        Some(request.as_str().to_owned()),
        "the exchange keeps its request"
    );
    assert_eq!(
        desk.request_row(&request).await,
        format!(
            "EXCHANGE_TRANSACTION|{id}|MAIN|{TELLER}|PENDING|confirm:{id}",
            id = t.id.0
        ),
        "§5.1 type; G3: the transaction's branch, the teller as proposer, key confirm:<id>"
    );
    assert_eq!(desk.requests().await, 1);
    assert_eq!(
        desk.quote_row(&q).await,
        format!("ACCEPTED|{}", t.id.0),
        "§4.4: the quote is accepted before the limit is asked (§5.3: it keeps its clock)"
    );
    desk.db.finish().await;
}

/// §5.2 `[proposed]`: the THB that changes hands. A BUY with a fee settles below its THB amount; a
/// SELL with a fee settles above it. If the owner answers Q-3 with `thb_amount`, this test is
/// re-expected by the ticket author, not by the implementer.
#[tokio::test]
async fn the_limit_compares_the_settle_amount_not_the_thb_amount() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    // BUY 100 USD at 32.10, fee 10: thb 3210.00, settle 3200.00
    let buy_quote = desk
        .quote("MAIN", Side::Buy, "100", Some("10.00"), now)
        .await;
    let buy = desk.draft(&buy_quote, now).await;
    // SELL 100 USD at 32.45, fee 10: thb 3245.00, settle 3255.00
    let sell_quote = desk
        .quote("MAIN", Side::Sell, "100", Some("10.00"), now)
        .await;
    let sell = desk.draft(&sell_quote, now).await;
    desk.limit("MAIN", "3205.00").await;

    assert_eq!(
        desk.confirm(buy.id, later(now, 10))
            .await
            .expect("confirmed"),
        ConfirmOutcome::Confirmed,
        "settle 3200.00 ≤ 3205.00, although thb 3210.00 is above it"
    );
    desk.limit("MAIN", "3250.00").await;
    assert!(
        matches!(
            desk.confirm(sell.id, later(now, 10)).await,
            Ok(ConfirmOutcome::PendingApproval(_))
        ),
        "settle 3255.00 > 3250.00, although thb 3245.00 is not"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn only_the_exchanges_own_branch_limit_counts() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (_, t) = desk.main_draft(now).await;
    desk.limit(BR2, "1000.00").await;

    assert_eq!(
        desk.confirm(t.id, later(now, 10)).await.expect("confirmed"),
        ConfirmOutcome::Confirmed,
        "BR2's limit says nothing about a MAIN exchange"
    );
    desk.db.finish().await;
}

// ---- confirm: the state and the quote ----

#[tokio::test]
async fn only_a_draft_is_confirmed() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, confirmed) = desk.main_draft(now).await;
    desk.confirm(confirmed.id, later(now, 10)).await.unwrap();
    let (_, cancelled) = desk.main_draft(now).await;
    desk.cancel(cancelled.id, "customer left", later(now, 5))
        .await
        .unwrap();
    let (_, pending) = desk.main_draft(now).await;
    desk.limit("MAIN", "1000.00").await;
    desk.confirm(pending.id, later(now, 10)).await.unwrap();

    for (t, was) in [
        (&confirmed, TransactionStatus::Confirmed),
        (&cancelled, TransactionStatus::Cancelled),
        (&pending, TransactionStatus::PendingApproval),
    ] {
        assert_eq!(
            code(desk.confirm(t.id, later(now, 20)).await),
            "staleVersion",
            "§4.4 confirm step 1: {was:?} is not DRAFT"
        );
        assert_eq!(desk.stored(t.id).await.status, was, "nothing moved");
    }
    assert_eq!(
        desk.requests().await,
        1,
        "a repeated confirm opens no second request"
    );
    assert_eq!(
        desk.quote_row(&q).await,
        format!("ACCEPTED|{}", confirmed.id.0)
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn a_quote_past_its_expiry_is_refused_at_confirm_and_nothing_moves() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;
    desk.limit("MAIN", "1000.00").await;

    assert_eq!(
        code(desk.confirm(t.id, q.expires_at).await),
        "quoteExpired",
        "D-5: valid only while now < expires_at"
    );
    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Draft);
    assert_eq!(
        desk.quote_row(&q).await,
        "ACTIVE|",
        "the quote is not accepted"
    );
    assert_eq!(desk.requests().await, 0, "no request either");
    desk.db.finish().await;
}

#[tokio::test]
async fn a_new_publication_at_the_branch_expires_the_quote_at_confirm() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;
    desk.db.publish("MAIN", &[USD]).await;

    assert_eq!(
        code(desk.confirm(t.id, later(now, 10)).await),
        "quoteExpired",
        "D-5: the quote's publication is no longer the branch's current one"
    );
    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Draft);
    assert_eq!(desk.quote_row(&q).await, "ACTIVE|");
    desk.db.finish().await;
}

// ---- confirm: the steps (D-7) ----

#[tokio::test]
async fn every_registered_step_runs_before_in_once_with_this_confirm_and_never_after_in() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (_, t) = desk.main_draft(now).await;
    let (first, first_calls) = Recorder::step(None);
    let (second, second_calls) = Recorder::step(None);
    let extras = SaveExtras {
        staff_draft_id: Some("draft-9".to_owned()),
        count_confirmed: true,
        kiosk_request_id: None,
    };

    desk.confirm_with(vec![first, second], t.id, &extras, later(now, 10))
        .await
        .expect("confirmed");

    let expected = StepCall {
        hook: "before_in",
        branch: "MAIN".to_owned(),
        extras: extras.clone(),
        actor: teller(),
        now: later(now, 10),
        had_prior: false,
    };
    assert_eq!(*first_calls.lock().unwrap(), vec![expected.clone()]);
    assert_eq!(
        *second_calls.lock().unwrap(),
        vec![expected],
        "after_in is the completion's (TX-5), not confirm's"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn a_step_that_refuses_ends_confirm_with_its_refusal_and_nothing_moves() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;
    desk.limit("MAIN", "1000.00").await;
    let (refuser, _) = Recorder::step(Some("countFirst: count the cash first"));

    assert_eq!(
        code(
            desk.confirm_with(vec![refuser], t.id, &SaveExtras::default(), later(now, 10))
                .await
        ),
        "countFirst",
        "D-7: the step's own refusal, unchanged"
    );
    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Draft);
    assert_eq!(
        desk.quote_row(&q).await,
        "ACTIVE|",
        "§4.4: the steps run (3) before the quote is accepted (4)"
    );
    assert_eq!(
        desk.requests().await,
        0,
        "and before the limit is asked (5)"
    );
    desk.db.finish().await;
}

// ---- cancel ----

#[tokio::test]
async fn a_draft_is_cancelled_with_its_reason_and_its_quote_is_cancelled() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;

    let answer = desk
        .cancel(t.id, "customer changed their mind", later(now, 30))
        .await
        .expect("cancelled");
    let stored = desk.stored(t.id).await;
    assert_eq!(answer, stored, "cancel answers the exchange as stored");
    assert_eq!(stored.status, TransactionStatus::Cancelled);
    assert_eq!(
        stored.cancel_reason.as_deref(),
        Some("customer changed their mind")
    );
    assert_eq!(stored.cancelled_at, Some(later(now, 30)));
    assert_eq!(
        stored.document_ref, None,
        "D-3: a cancelled draft burns no number"
    );
    assert_eq!(
        desk.quote_row(&q).await,
        "CANCELLED|",
        "§9: cancel sets the quote CANCELLED"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn a_confirmed_exchange_is_cancelled_and_its_accepted_quote_is_cancelled() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;
    desk.confirm(t.id, later(now, 10)).await.unwrap();

    desk.cancel(t.id, "drawer short", later(now, 30))
        .await
        .expect("CONFIRMED ⇒ CANCELLED (§4.1)");
    let stored = desk.stored(t.id).await;
    assert_eq!(stored.status, TransactionStatus::Cancelled);
    assert_eq!(stored.cancel_reason.as_deref(), Some("drawer short"));
    assert_eq!(desk.quote_row(&q).await, "CANCELLED|");
    desk.db.finish().await;
}

#[tokio::test]
async fn an_exchange_waiting_for_approval_is_not_cancelled_by_the_desk() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;
    desk.limit("MAIN", "1000.00").await;
    let Ok(ConfirmOutcome::PendingApproval(request)) = desk.confirm(t.id, later(now, 10)).await
    else {
        panic!("waits for approval");
    };

    assert_eq!(
        code(desk.cancel(t.id, "too slow", later(now, 30)).await),
        "staleVersion",
        "§9: PENDING_APPROVAL leaves by the manager's decision only"
    );
    let stored = desk.stored(t.id).await;
    assert_eq!(stored.status, TransactionStatus::PendingApproval);
    assert_eq!(stored.cancel_reason, None);
    assert!(desk.request_row(&request).await.contains("|PENDING|"));
    assert_eq!(desk.quote_row(&q).await, format!("ACCEPTED|{}", t.id.0));
    desk.db.finish().await;
}

#[tokio::test]
async fn a_cancelled_exchange_is_not_cancelled_again() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (_, t) = desk.main_draft(now).await;
    desk.cancel(t.id, "first", later(now, 30)).await.unwrap();

    assert_eq!(
        code(desk.cancel(t.id, "second", later(now, 40)).await),
        "staleVersion"
    );
    let stored = desk.stored(t.id).await;
    assert_eq!(stored.cancel_reason.as_deref(), Some("first"));
    assert_eq!(stored.cancelled_at, Some(later(now, 30)));
    desk.db.finish().await;
}

#[tokio::test]
async fn a_cancel_without_a_reason_is_refused_and_nothing_moves() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;

    for reason in ["", "   "] {
        assert_eq!(
            code(desk.cancel(t.id, reason, later(now, 30)).await),
            "validationFailed",
            "§4.2: cancel takes `{{ reason }}` ({reason:?})"
        );
    }
    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Draft);
    assert_eq!(desk.quote_row(&q).await, "ACTIVE|");
    desk.db.finish().await;
}

/// A draft whose quote has expired is still cancelled; the quote's stored ACTIVE becomes CANCELLED
/// (D-5: expiry is derived, nothing swept it).
#[tokio::test]
async fn a_draft_whose_quote_expired_is_still_cancelled() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = desk.main_draft(now).await;

    desk.cancel(t.id, "quote ran out", later(q.expires_at, 60))
        .await
        .expect("cancelled");
    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Cancelled);
    assert_eq!(desk.quote_row(&q).await, "CANCELLED|");
    desk.db.finish().await;
}
