//! Ticket #873 (TX-5) — `CompleteExchange::run_in` (design `docs/design/blueprint/quote-transaction.md` §4.5,
//! §9.1 TX-5; `ledger-inventory.md` §4.3; `membership-points.md` §6.2).
//! Written before the implementation by the up-front test writer (claude/07d3765f, on the xchange BE lane
//! controller claude/ad05bf8d's order, 2026-10-11). Do not edit, delete or weaken; add tests in a file of your
//! own.
//!
//! The lane controller's rulings this file follows (all on #873):
//! - Q1: the posting port is #924's `CashPostings`, injected; a test double records each `post_in` call.
//! - Q2 / Q5: the costs the posting answers are written on the line; a negative margin reads back through
//!   `TransactionStore::find_in` (the line's margin becomes a signed `MoneyDelta`).
//! - Q3 / Q4: the effects are `ExchangeEffects::new(Vec<Arc<dyn ExchangeEffect>>)`, §6.2's trait; each runs once,
//!   in the completion's transaction, and an `Err` from one fails the completion.
//! - Q6: two legs per line. The foreign leg carries `line: Some(line.id)`; the THB leg carries `settle_thb` and
//!   `line: None`, at `Drawer` for Cash and `Bank` otherwise. Both name the source `EXCHANGE`/`<id>`/`complete`.
//! - Q7: `allow_negative` is always `None`; `stockInsufficient` fails the completion and nothing is written.
//! - Q8: `CompleteExchange::new(pool, files, postings, steps, effects)`; the receipt (revision 1) names a
//!   `files` row of the branch, and its number is the exchange's `document_ref`.
//!
//! Read from the design, not separately ruled (the ticket author's reading; object on #873 if wrong):
//! - A DEMO exchange has no shift (§4.4 step 2), so it posts nothing; today's save moves stock only for LIVE
//!   (`save_transaction.rs:413`). It still completes with a number and a receipt.
//! - "Every step's `after_in` runs" (§4.5, #873 §7 step 7): each registered step's `after_in` once, and no
//!   `before_in` (that ran at confirm).
//!
//! Each call runs in a transaction the test opens and **commits whatever the answer**, as `tx4_common` does, so a
//! refusal that wrote half of its work is seen in the rows read back. The rollback test rolls back instead.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips. Each test
//! works in a schema of its own.

mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::Arc;

use pos_error::AppError;
use pos_infra::transactions::Transactor;
use quote_common::*;
use tx4_common::*;
use xc_approval::domain::request::Outcome;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_cash::domain::posting::CashPlace;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_exchange::domain::transaction::transaction::TransactionStatus;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, MoneyDelta, Side};
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;
use xchange::application::settings::PaymentSettingsService;
use xchange::application::workflows::exchange::complete::EffectPhase;
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;
use xchange::application::workflows::payment_save_step::{PaymentSaveStep, UNAVAILABLE};
use xchange::application::workflows::save_step::{SaveContext, SaveExtras, SaveStep};
use xchange::infrastructure::settings::branch_setting_store::BranchSettingStore;

use tx5_common::*;

// ------------------------------------------------------------ §9.1: COMPLETED, number, receipt revision 1

#[tokio::test]
async fn complete_gives_completed_a_document_ref_and_receipt_revision_1() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);

    let done = c
        .complete(&ledger, t.id, later(now, 5))
        .await
        .expect("a CONFIRMED exchange completes");
    assert_eq!(done.transaction.id, t.id);
    assert_eq!(done.transaction.status, TransactionStatus::Completed);
    let document_ref = done
        .transaction
        .document_ref
        .clone()
        .expect("a COMPLETED exchange has its document_ref");
    assert_eq!(done.transaction.completed_at, Some(later(now, 5)));
    assert_eq!(done.receipt.revision, 1, "receipt revision 1");
    assert_eq!(
        done.receipt.number, document_ref,
        "Q8: the receipt's number is the document_ref"
    );

    let stored = c.desk.stored(t.id).await;
    assert_eq!(stored.status, TransactionStatus::Completed);
    assert_eq!(stored.document_ref.as_deref(), Some(document_ref.as_str()));
    assert_eq!(
        stored.receipts,
        vec![done.receipt.clone()],
        "one receipt, the one answered"
    );
    assert_eq!(
        c.desk
            .db
            .count(&format!(
                "SELECT count(*) FROM files WHERE key = '{}' AND branch_id = 'MAIN'",
                done.receipt.document_ref
            ))
            .await,
        1,
        "Q8: the receipt names a `files` row of the branch"
    );
    c.finish().await;
}

// ------------------------------------------------------------ the posting (ledger §4.3, Q1, Q6, Q7)

#[tokio::test]
async fn a_cash_buy_posts_both_legs_once_customer_to_drawer_and_drawer_to_customer() {
    let c = Counter::start().await;
    let now = whole_second_now();
    // a fee, so that settle_thb (Q6) differs from thb_amount
    let q = c
        .desk
        .quote("MAIN", Side::Buy, "100", Some("50.00"), now)
        .await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    let line = &t.lines[0];
    assert_ne!(
        line.settle_thb, line.thb_amount,
        "the fee separates the two"
    );
    let ledger = Ledger::new("3210.00", None);

    c.complete(&ledger, t.id, later(now, 5))
        .await
        .expect("completes");
    let calls = ledger.calls();
    assert_eq!(calls.len(), 1, "one post_in per completion: {calls:?}");
    let posted = &calls[0];
    assert_eq!(
        posted.reqs.len(),
        2,
        "both legs in one posting: {:?}",
        posted.reqs
    );
    assert_eq!(posted.allow_negative, None, "Q7: never allow_negative");

    let drawer = CashPlace::Drawer(c.shift.clone());
    let foreign = leg(posted, usd());
    assert_eq!(
        foreign.from,
        CashPlace::Customer,
        "BUY: the foreign comes from the customer"
    );
    assert_eq!(
        foreign.to, drawer,
        "into the drawer of the exchange's shift"
    );
    assert_eq!(foreign.quantity.value(), line.quantity.value());
    assert_eq!(
        foreign.line,
        Some(line.id),
        "Q6: the foreign leg names the line"
    );
    assert_eq!(foreign.reverses, None);
    assert_eq!(foreign.source, source_of(&t));

    let thb = leg(posted, CurrencyCode::THB);
    assert_eq!(thb.from, drawer, "BUY by cash: the baht leaves the drawer");
    assert_eq!(thb.to, CashPlace::Customer);
    assert_eq!(
        thb.quantity.value(),
        thb_quantity(line.settle_thb),
        "Q6: settle_thb"
    );
    assert_eq!(thb.line, None, "Q6: the THB leg names no line");
    assert_eq!(thb.reverses, None);
    assert_eq!(thb.source, source_of(&t), "one source for both legs");
    c.finish().await;
}

#[tokio::test]
async fn a_cash_sell_posts_drawer_to_customer_and_customer_to_drawer() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3000.00", Some(MoneyDelta::from_minor(24_500)));

    c.complete(&ledger, t.id, later(now, 5))
        .await
        .expect("completes");
    let calls = ledger.calls();
    assert_eq!(calls.len(), 1);
    let drawer = CashPlace::Drawer(c.shift.clone());
    let foreign = leg(&calls[0], usd());
    assert_eq!(
        (foreign.from.clone(), foreign.to.clone()),
        (drawer.clone(), CashPlace::Customer)
    );
    let thb = leg(&calls[0], CurrencyCode::THB);
    assert_eq!(
        (thb.from.clone(), thb.to.clone()),
        (CashPlace::Customer, drawer)
    );
    assert_eq!(thb.quantity.value(), thb_quantity(t.lines[0].settle_thb));
    c.finish().await;
}

#[tokio::test]
async fn a_non_cash_payment_takes_the_baht_through_the_bank_not_the_drawer() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c
        .live(Side::Buy, PaymentMethod::BankTransfer, false, now)
        .await;
    let ledger = Ledger::new("3210.00", None);

    c.complete(&ledger, t.id, later(now, 5))
        .await
        .expect("a non-cash exchange completes without a cash count");
    let calls = ledger.calls();
    let thb = leg(&calls[0], CurrencyCode::THB);
    assert_eq!(
        (thb.from.clone(), thb.to.clone()),
        (CashPlace::Bank, CashPlace::Customer),
        "ledger §4.3: THB at Bank for a non-cash payment"
    );
    let foreign = leg(&calls[0], usd());
    assert_eq!(foreign.to, CashPlace::Drawer(c.shift.clone()));
    c.finish().await;
}

#[tokio::test]
async fn the_costs_the_posting_answers_are_written_on_the_line() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let buy = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    c.complete(&Ledger::new("3210.00", None), buy.id, later(now, 5))
        .await
        .expect("completes");
    assert_eq!(
        c.line_costs(buy.id).await,
        "3210.00|NULL",
        "Q2: a BUY's cost, and no margin (ledger §4.3)"
    );
    let stored = c.desk.stored(buy.id).await;
    assert_eq!(stored.lines[0].cost_basis_thb, Some(money("3210.00")));
    c.finish().await;
}

/// Q5: a SELL below cost has a negative margin; it is written as the posting answers it, and the exchange still
/// reads back through `TransactionStore::find_in`.
#[tokio::test]
async fn a_sell_below_cost_keeps_its_negative_margin_and_reads_back() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3265.00", Some(MoneyDelta::from_minor(-2_000)));

    let done = c
        .complete(&ledger, t.id, later(now, 5))
        .await
        .expect("completes");
    assert_eq!(done.transaction.status, TransactionStatus::Completed);
    assert_eq!(c.line_costs(t.id).await, "3265.00|-20.00");
    let stored = c.desk.stored(t.id).await;
    assert_eq!(stored.status, TransactionStatus::Completed);
    assert_eq!(
        stored.lines[0].realized_margin_thb.map(|m| m.to_string()),
        Some("-20.00".to_owned()),
        "Q5: the signed margin reads back"
    );
    c.finish().await;
}

/// Read from §4.4 step 2 and today's save (`save_transaction.rs:413`): a DEMO exchange has no shift and moves no
/// stock, so it posts nothing; it still completes.
#[tokio::test]
async fn a_demo_exchange_completes_and_posts_nothing() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let (_, t) = c.desk.main_draft(now).await;
    assert_eq!(
        c.desk.confirm(t.id, later(now, 1)).await.expect("confirm"),
        ConfirmOutcome::Confirmed
    );
    let ledger = Ledger::new("3210.00", None);

    let done = c
        .complete(&ledger, t.id, later(now, 5))
        .await
        .expect("a DEMO exchange completes");
    assert_eq!(done.transaction.status, TransactionStatus::Completed);
    assert!(done.transaction.document_ref.is_some());
    assert!(
        ledger.calls().is_empty(),
        "DEMO posts nothing: {:?}",
        ledger.calls()
    );
    c.finish().await;
}

// ------------------------------------------------------------ §9.1: what refuses, and writes nothing

#[tokio::test]
async fn only_a_confirmed_exchange_completes() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let ledger = Ledger::new("3210.00", None);

    // DRAFT
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let draft = c.live_draft(&q, PaymentMethod::Cash, true, now).await;
    // CANCELLED
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let cancelled = c.live_draft(&q, PaymentMethod::Cash, true, now).await;
    c.desk
        .cancel(cancelled.id, "changed mind", later(now, 1))
        .await
        .expect("cancel a DRAFT");
    // PENDING_APPROVAL
    c.desk.limit("MAIN", "1000.00").await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let pending = c.live_draft(&q, PaymentMethod::Cash, true, now).await;
    assert!(matches!(
        c.desk.confirm(pending.id, later(now, 1)).await,
        Ok(ConfirmOutcome::PendingApproval(_))
    ));

    for (what, t) in [
        ("DRAFT", &draft),
        ("CANCELLED", &cancelled),
        ("PENDING_APPROVAL", &pending),
    ] {
        let before = c.status(t.id).await;
        assert_eq!(
            code(c.complete(&ledger, t.id, later(now, 5)).await),
            "staleVersion",
            "a {what} exchange is not completed"
        );
        assert_eq!(c.status(t.id).await, before, "{what} stays {what}");
        assert_eq!(
            c.written(t.id).await,
            nothing_written(),
            "{what}: nothing written"
        );
    }
    assert!(
        ledger.calls().is_empty(),
        "nothing posted: {:?}",
        ledger.calls()
    );
    c.finish().await;
}

#[tokio::test]
async fn a_completed_exchange_is_not_completed_again() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);
    let done = c
        .complete(&ledger, t.id, later(now, 5))
        .await
        .expect("completes");

    assert_eq!(
        code(c.complete(&ledger, t.id, later(now, 6)).await),
        "staleVersion"
    );
    assert_eq!(ledger.calls().len(), 1, "posted once");
    assert_eq!(c.written(t.id).await.0, 1);
    assert_eq!(c.written(t.id).await.1, 1, "one receipt");
    assert_eq!(
        c.desk.stored(t.id).await.document_ref,
        done.transaction.document_ref
    );
    c.finish().await;
}

#[tokio::test]
async fn an_expired_quote_is_quote_expired_and_nothing_is_written() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);

    let past = q.expires_at + chrono::Duration::seconds(1);
    assert_eq!(code(c.complete(&ledger, t.id, past).await), "quoteExpired");
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    assert!(ledger.calls().is_empty());
    c.finish().await;
}

/// §9.1: an ACCEPTED quote past `expires_at` (a late approval) is `quoteExpired`.
#[tokio::test]
async fn a_late_approval_past_the_quotes_expiry_is_quote_expired() {
    let c = Counter::start().await;
    let now = whole_second_now();
    c.desk.limit("MAIN", "1000.00").await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let t = c.live_draft(&q, PaymentMethod::Cash, true, now).await;
    let request = match c.desk.confirm(t.id, later(now, 1)).await {
        Ok(ConfirmOutcome::PendingApproval(r)) => r,
        other => panic!("above the limit it waits: {other:?}"),
    };
    c.desk
        .decide(
            &request,
            &staff(MANAGER, Role::Manager, "MAIN"),
            Outcome::Approved,
            "late",
        )
        .await
        .expect("the manager approves");
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    let ledger = Ledger::new("3210.00", None);

    let past = q.expires_at + chrono::Duration::seconds(1);
    assert_eq!(code(c.complete(&ledger, t.id, past).await), "quoteExpired");
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    c.finish().await;
}

/// §9.1: the same after a new publication (D-5), before `expires_at`.
#[tokio::test]
async fn a_new_publication_makes_the_quote_expired() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    c.desk.db.publish("MAIN", &[USD]).await;
    let ledger = Ledger::new("3210.00", None);

    assert!(later(now, 5) < q.expires_at, "still before expires_at");
    assert_eq!(
        code(c.complete(&ledger, t.id, later(now, 5)).await),
        "quoteExpired"
    );
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    c.finish().await;
}

#[tokio::test]
async fn cash_needs_the_count_confirmed() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, false, now).await;
    let ledger = Ledger::new("3210.00", None);

    match c.complete(&ledger, t.id, later(now, 5)).await {
        Err(AppError::Domain(m)) => assert_eq!(m, COUNT_FIRST),
        other => panic!("cash without countConfirmed is refused with today's sentence: {other:?}"),
    }
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    assert!(ledger.calls().is_empty());
    c.finish().await;
}

#[tokio::test]
async fn the_exchanges_shift_must_still_be_open() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    c.desk
        .db
        .exec(&format!(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE id = '{}'",
            c.shift
        ))
        .await;
    let ledger = Ledger::new("3210.00", None);

    match c.complete(&ledger, t.id, later(now, 5)).await {
        Err(AppError::Domain(m)) => assert_eq!(m, NO_OPEN_SHIFT),
        other => panic!("a closed shift is refused with today's sentence: {other:?}"),
    }
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    assert!(ledger.calls().is_empty());
    c.finish().await;
}

#[tokio::test]
async fn insufficient_stock_is_stock_insufficient_and_nothing_is_written() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::refusing();
    let (redeem, _) = Effect::of("redeem", EffectPhase::Redeem, false);
    let (outbox, _) = Effect::of("outbox", EffectPhase::Outbox, false);
    let service = c.service(&ledger, Vec::new(), vec![redeem, outbox]);
    let requests = "SELECT count(*) FROM approval_requests";
    let requests_before = c.desk.db.count(requests).await;

    assert_eq!(
        code(c.complete_with(&service, t.id, later(now, 5)).await),
        "stockInsufficient"
    );
    assert_eq!(
        c.desk.db.count(requests).await,
        requests_before,
        "Q7 (ruling issuecomment-6108290812 item 2): no approval request is opened"
    );
    assert_eq!(ledger.calls().len(), 1);
    assert_eq!(ledger.calls()[0].allow_negative, None, "Q7");
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    c.finish().await;
}

// ------------------------------------------------------------ steps and effects, once, on the caller's tx

#[tokio::test]
async fn every_steps_after_in_runs_once_and_no_before_in() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let (step, calls) = Recorder::step(None);
    let service = c.service(&Ledger::new("3210.00", None), vec![step], Vec::new());

    c.complete_with(&service, t.id, later(now, 5))
        .await
        .expect("completes");
    let calls = calls.lock().unwrap().clone();
    assert_eq!(
        calls.iter().map(|c| c.hook).collect::<Vec<_>>(),
        vec!["after_in"],
        "§4.5: the steps' after_in once; before_in ran at confirm"
    );
    assert_eq!(calls[0].branch, "MAIN");
    assert_eq!(calls[0].actor, teller());
    assert!(!calls[0].had_prior);
    assert_eq!(calls[0].now, later(now, 5));
    c.finish().await;
}

#[tokio::test]
async fn each_registered_effect_runs_once_with_the_completions_facts() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let (redeem, redeemed) = Effect::of("redeem", EffectPhase::Redeem, false);
    let (outbox, outboxed) = Effect::of("outbox", EffectPhase::Outbox, false);
    let service = c.service(
        &Ledger::new("3210.00", None),
        Vec::new(),
        vec![redeem, outbox],
    );

    c.complete_with(&service, t.id, later(now, 5))
        .await
        .expect("completes");
    for (what, seen) in [("Redeem", &redeemed), ("Outbox", &outboxed)] {
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "the {what} effect runs once: {seen:?}");
        let f = &seen[0];
        assert_eq!(f.transaction_id, t.id.0);
        assert_eq!(f.organization_id, t.organization_id);
        assert_eq!(f.branch_id.as_str(), "MAIN");
        assert_eq!(Some(f.quote_id), t.quote_id);
        assert_eq!(f.customer_id.as_deref(), Some(CUSTOMER));
        assert_eq!(f.mode, Mode::Live);
        assert_eq!(f.completed_at, later(now, 5));
        assert_eq!(
            f.business_date, t.value_date,
            "the business date is the value date"
        );
    }
    assert_eq!(
        c.written(t.id).await.2,
        2,
        "both wrote on the completion's transaction"
    );
    c.finish().await;
}

#[tokio::test]
async fn a_refusing_effect_fails_the_completion() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let (redeem, _) = Effect::of("redeem", EffectPhase::Redeem, true);
    let service = c.service(&Ledger::new("3210.00", None), Vec::new(), vec![redeem]);

    let mut tx = c.desk.db.pool.begin().await.unwrap();
    let got = service
        .run_in(&mut tx, t.id, &teller(), later(now, 5))
        .await;
    assert_eq!(
        code(got),
        "refused",
        "the effect's Err is the completion's answer"
    );
    // §6.2: the caller rolls its transaction back on the Err
    tx.rollback().await.unwrap();
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    c.finish().await;
}

/// Everything a completion writes is on the caller's transaction: rolled back, nothing is left.
#[tokio::test]
async fn a_completion_rolled_back_by_its_caller_leaves_nothing() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    let (outbox, _) = Effect::of("outbox", EffectPhase::Outbox, false);
    let service = c.service(
        &Ledger::new("3000.00", Some(MoneyDelta::from_minor(24_500))),
        Vec::new(),
        vec![outbox],
    );

    let mut tx = c.desk.db.pool.begin().await.unwrap();
    let done = service
        .run_in(&mut tx, t.id, &teller(), later(now, 5))
        .await
        .expect("completes on the caller's transaction");
    let receipt_file = done.receipt.document_ref.clone();
    tx.rollback().await.unwrap();

    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    assert_eq!(c.line_costs(t.id).await, "NULL|NULL");
    assert_eq!(
        c.desk
            .db
            .count(&format!(
                "SELECT count(*) FROM files WHERE key = '{receipt_file}'"
            ))
            .await,
        0,
        "the receipt's files row rolls back too"
    );
    c.finish().await;
}

// ------------------------------------------------------------ PaymentSaveStep's check moves to before_in

/// §4.5: `PaymentSaveStep` refuses in `before_in` (it writes nothing), so a confirm refuses before anything is
/// written. PromptPay is off for BUY in the defaults (`payment-settings.mjs:3`, `payment_save_step_test.rs`).
#[tokio::test]
async fn the_payment_step_refuses_a_live_method_that_is_off_in_before_in() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let t = c.live_draft(&q, PaymentMethod::PromptPay, false, now).await;
    let pool = c.desk.db.pool.clone();
    let step = PaymentSaveStep::new(PaymentSettingsService::new(
        BranchTx::new(Transactor::new(pool.clone())),
        BranchSettingStore::new(pool.clone()),
        PgBranchRepository::new(pool.clone()),
    ));
    let form = TransactionForm::from_transaction(&t).expect("stored form");
    let extras = SaveExtras::default();
    let actor = teller();
    let ctx = SaveContext {
        branch: &t.branch_id,
        form: &form,
        extras: &extras,
        booking: None,
        actor: &actor,
        teller: None,
        now,
        prior: None,
    };
    let mut tx = pool.begin().await.unwrap();
    match step.before_in(&mut tx, &ctx).await {
        Err(AppError::Domain(m)) => assert_eq!(m, UNAVAILABLE),
        other => panic!("before_in refuses a LIVE BUY by PromptPay: {other:?}"),
    }
    tx.rollback().await.unwrap();

    // and so confirm refuses it, leaving the DRAFT
    let confirmed = c
        .desk
        .confirm_with(vec![Arc::new(step)], t.id, &extras, later(now, 1))
        .await;
    match confirmed {
        Err(AppError::Domain(m)) => assert_eq!(m, UNAVAILABLE),
        other => panic!("confirm runs before_in: {other:?}"),
    }
    assert_eq!(c.status(t.id).await, "DRAFT");
    c.finish().await;
}

// ------------------------------------------------------------ #873 ruling issuecomment-6108290812, item 3

/// Item 3 (§4.5's order, `membership-points.md` §6.2): the Redeem effect runs before the exchange is COMPLETED,
/// the Outbox effect after, both on the completion's transaction.
#[tokio::test]
async fn redeem_runs_before_completed_and_outbox_after() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let (redeem, _, redeem_saw) = Effect::watching("redeem", EffectPhase::Redeem, false);
    let (outbox, _, outbox_saw) = Effect::watching("outbox", EffectPhase::Outbox, false);
    let service = c.service(
        &Ledger::new("3210.00", None),
        Vec::new(),
        vec![redeem, outbox],
    );

    c.complete_with(&service, t.id, later(now, 5))
        .await
        .expect("completes");
    assert_eq!(
        redeem_saw.lock().unwrap().clone(),
        vec!["CONFIRMED".to_owned()],
        "Redeem runs before the exchange is marked COMPLETED"
    );
    assert_eq!(
        outbox_saw.lock().unwrap().clone(),
        vec!["COMPLETED".to_owned()],
        "Outbox runs after the exchange is COMPLETED"
    );
    c.finish().await;
}
