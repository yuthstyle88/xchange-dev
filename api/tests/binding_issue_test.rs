//! Up-front tests for BQ-3 `IssueBindingQuote` (design `docs/design/blueprint/binding-quote.md` D-4, D-6, D-9,
//! D-10, D-14, D-15, D-17; §4.2 Q1–Q5, Q7, Q16, Q19–Q23, Q25, Q26). Written by the sub-controller claude/644b3958
//! on `feat/binding-quote` before the implementation. **Do not edit, delete or weaken.**
//!
//! `IssueBindingQuote::run_in` is called as the command seam calls it: committed on `Ok` (`Issued` and
//! `ApprovalRequired` alike) and rolled back on `Err`, then the rows are read back. So an `ApprovalRequired`
//! answer must leave **no quote and no reservation** in the caller's transaction (D-15).
//! The approval core is the real registry (`make_approval_service`), so `BINDING_QUOTE` must be registered and
//! decided by BQ-6's subject.
//!
//! Not here: Q6 (the `Idempotency-Key` replay is the command seam's: `v3_binding_quote_issue_test.rs`); Q24 (a
//! booking never touches a binding quote: nothing in the booking code names `quotes.type`, and BQ-3 writes no
//! booking row); a quote of two lines (`QuoteService` refuses more than one line, so Q4 is pinned as a second
//! lock over what the first left).
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`) and BQ-0's migration.

mod binding_issue_common;
mod quote_common;
mod tx4_common;

use binding_issue_common::*;
use chrono::Duration;
use quote_common::*;
use rust_decimal::Decimal;
use tx4_common::*;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_exchange::domain::quote::binding_policy::{DESK_ONLY, NOT_ENABLED, NOT_VERIFIED};
use xc_exchange::domain::quote::quote::{QuoteChannel, QuoteStatus};
use xc_exchange::domain::quote::quote_type::QuoteType;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::binding_approval::proposal_id;
use xchange::application::workflows::exchange::binding_issue::{APPROVAL_REJECTED, STALE_PROPOSAL};

const MANAGER: &str = "employee-manager";

/// A verification case: `status`, `expires_at`.
type Case = (&'static str, Option<chrono::DateTime<chrono::Utc>>);

#[tokio::test]
async fn q1_no_lifetime_entered_refuses_and_writes_nothing() {
    let desk = desk("300", None).await;
    let now = whole_second_now();

    let got = issue(&desk, request(Side::Sell, "100"), now).await;

    assert_eq!(
        refusal(got),
        NOT_ENABLED,
        "D-10: no lifetime ⇒ binding quotes are off"
    );
    assert_eq!(quotes(&desk).await, 0, "no quote");
    assert_eq!(reservations(&desk).await, 0, "no reservation");
    desk.db.finish().await;
}

#[tokio::test]
async fn q2_a_sell_line_is_locked_for_the_lifetime_and_reserved_until_the_same_instant() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();

    let q = issued(issue(&desk, request(Side::Sell, "100"), now).await);

    assert_eq!(q.quote.quote_type, QuoteType::Binding);
    assert_eq!(q.quote.status, QuoteStatus::Active);
    assert_eq!(q.quote.channel, QuoteChannel::Desk);
    assert_eq!(
        q.quote.expires_at,
        now + Duration::seconds(TTL_SECONDS),
        "D-10"
    );
    assert_eq!(quote_row(&desk, q.quote.id).await, "BINDING|ACTIVE|DESK");
    assert_eq!(
        reservation_rows(&desk, q.quote.id).await,
        vec!["RESERVED|-|100|true".to_owned()],
        "D-4: one RESERVED row of 100 with the quote's expires_at"
    );
    assert_eq!(q.lines.len(), 1);
    assert_eq!(q.lines[0].quote_line_id, q.quote.lines[0].id);
    assert_eq!(q.lines[0].reserved_quantity, qty("100"), "D-12");
    desk.db.finish().await;
}

#[tokio::test]
async fn q3_a_sell_line_over_what_is_available_is_refused_whole() {
    let desk = desk("60", Some(TTL_SECONDS)).await;
    let now = whole_second_now();

    let got = issue(&desk, request(Side::Sell, "100"), now).await;

    assert_eq!(code(got), "stockInsufficient", "D-4");
    assert_eq!(quotes(&desk).await, 0, "no quote");
    assert_eq!(reservations(&desk).await, 0, "no reservation");
    desk.db.finish().await;
}

#[tokio::test]
async fn q4_a_second_lock_over_what_the_first_left_is_refused_and_the_first_stays() {
    let desk = desk("150", Some(TTL_SECONDS)).await;
    let now = whole_second_now();
    let first = issued(issue(&desk, request(Side::Sell, "100"), now).await);

    let got = issue(&desk, request(Side::Sell, "100"), later(now, 1)).await;

    assert_eq!(
        code(got),
        "stockInsufficient",
        "D-4: 150 − 100 reserved leaves 50"
    );
    assert_eq!(quotes(&desk).await, 1, "only the first quote");
    assert_eq!(reservations(&desk).await, 1, "only the first reservation");
    assert_eq!(
        reservation_rows(&desk, first.quote.id).await,
        vec!["RESERVED|-|100|true".to_owned()]
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q5_a_buy_line_is_issued_with_no_reservation() {
    let desk = desk("0", Some(TTL_SECONDS)).await;
    let now = whole_second_now();

    let q = issued(issue(&desk, request(Side::Buy, "100"), now).await);

    assert_eq!(q.quote.quote_type, QuoteType::Binding);
    assert_eq!(quote_row(&desk, q.quote.id).await, "BINDING|ACTIVE|DESK");
    assert_eq!(
        reservations(&desk).await,
        0,
        "B05 D-15: a BUY line reserves nothing"
    );
    assert_eq!(q.lines.len(), 1);
    assert_eq!(q.lines[0].reserved_quantity, qty("0"), "D-12: nothing held");
    desk.db.finish().await;
}

#[tokio::test]
async fn q7_a_changed_quantity_is_a_new_quote_and_the_replaced_one_is_cancelled_and_released() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();
    let old = issued(issue(&desk, request(Side::Sell, "100"), now).await);

    let mut change = request(Side::Sell, "150");
    change.replaces = Some(old.quote.id);
    let new = issued(issue(&desk, change, later(now, 5)).await);

    assert_ne!(new.quote.id, old.quote.id, "D-6: never edited");
    assert_eq!(
        quote_row(&desk, old.quote.id).await,
        "BINDING|CANCELLED|DESK"
    );
    assert_eq!(
        reservation_rows(&desk, old.quote.id).await,
        vec!["RELEASED|CANCELLED|100|true".to_owned()],
        "D-5: released at once"
    );
    assert_eq!(quote_row(&desk, new.quote.id).await, "BINDING|ACTIVE|DESK");
    assert_eq!(
        reservation_rows(&desk, new.quote.id).await,
        vec!["RESERVED|-|150|true".to_owned()]
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q7_replacing_a_quote_that_is_not_an_active_binding_quote_is_refused() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();
    let indicative = desk.quote("MAIN", Side::Sell, "100", None, now).await;

    let mut change = request(Side::Sell, "150");
    change.replaces = Some(indicative.id);
    let got = issue(&desk, change, later(now, 5)).await;

    assert_eq!(code(got), "validationFailed");
    assert_eq!(quotes(&desk).await, 1, "only the indicative quote");
    assert_eq!(
        quote_row(&desk, indicative.id).await,
        "INDICATIVE|ACTIVE|DESK",
        "the named quote is untouched"
    );
    assert_eq!(reservations(&desk).await, 0);
    desk.db.finish().await;
}

#[tokio::test]
async fn q16_app_kiosk_and_booking_are_refused_on_channel_and_write_nothing() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();

    for channel in [
        QuoteChannel::App,
        QuoteChannel::Kiosk,
        QuoteChannel::Booking,
    ] {
        let mut req = request(Side::Sell, "100");
        req.quote.channel = channel;
        let got = issue(&desk, req, now).await;
        assert_eq!(refusal(got), DESK_ONLY, "D-14: {channel:?}");
    }
    assert_eq!(quotes(&desk).await, 0, "no quote");
    assert_eq!(reservations(&desk).await, 0, "no reservation");
    desk.db.finish().await;
}

#[tokio::test]
async fn q19_under_the_branch_limit_or_with_no_limit_issues_with_no_request() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();

    // no limit at MAIN
    issued(issue(&desk, request(Side::Sell, "100"), now).await);
    // 100 × 32.45 = 3245.00 ≤ 10000.00
    desk.limit("MAIN", "10000.00").await;
    issued(issue(&desk, request(Side::Sell, "100"), later(now, 1)).await);

    assert_eq!(quotes(&desk).await, 2);
    assert!(
        binding_requests(&desk).await.is_empty(),
        "D-15: no approval request"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q20_over_the_branch_limit_opens_one_pending_request_and_writes_no_quote() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    desk.limit("MAIN", "1000.00").await;
    let now = whole_second_now();
    let req = request(Side::Sell, "100");
    let proposal = proposal_id(&req.quote, None);

    let id = approval_required(issue(&desk, req.clone(), now).await);

    assert_eq!(
        quotes(&desk).await,
        0,
        "D-15: no quote, though the answer was committed"
    );
    assert_eq!(reservations(&desk).await, 0, "D-15: no reservation");
    assert_eq!(
        binding_requests(&desk).await,
        vec![format!("BINDING_QUOTE|{proposal}|MAIN|{TELLER}|PENDING")],
        "one PENDING request naming the proposal"
    );
    let stored: String = desk
        .db
        .text("SELECT id::text FROM approval_requests WHERE type = 'BINDING_QUOTE'")
        .await
        .unwrap();
    assert_eq!(id, stored, "the answer names the request");

    // the same proposal again, before a decision: the same request, nothing more
    let again = approval_required(issue(&desk, req, later(now, 5)).await);
    assert_eq!(again, id);
    assert_eq!(binding_requests(&desk).await.len(), 1);
    assert_eq!(quotes(&desk).await, 0);
    desk.db.finish().await;
}

/// Over MAIN's 1000.00 limit: the PENDING request of `request(Sell, "100")`.
async fn pending(desk: &Desk) -> RequestId {
    desk.limit("MAIN", "1000.00").await;
    let id = approval_required(issue(desk, request(Side::Sell, "100"), whole_second_now()).await);
    RequestId::new(id)
}

#[tokio::test]
async fn q20_a_retry_naming_the_still_pending_request_issues_nothing() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let request_id = pending(&desk).await;

    let mut retry = request(Side::Sell, "100");
    retry.approval_request_id = Some(request_id.as_str().to_owned());
    let again = approval_required(issue(&desk, retry, whole_second_now()).await);

    assert_eq!(again, request_id.as_str());
    assert_eq!(quotes(&desk).await, 0);
    assert_eq!(reservations(&desk).await, 0);
    desk.db.finish().await;
}

#[tokio::test]
async fn q21_the_retry_naming_the_approved_request_issues_with_its_reservation() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let request_id = pending(&desk).await;
    desk.decide(
        &request_id,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Approved,
        "known customer",
    )
    .await
    .expect("O-5 A: the branch's manager decides");

    let now = whole_second_now();
    let mut retry = request(Side::Sell, "100");
    retry.approval_request_id = Some(request_id.as_str().to_owned());
    let q = issued(issue(&desk, retry, now).await);

    assert_eq!(quote_row(&desk, q.quote.id).await, "BINDING|ACTIVE|DESK");
    assert_eq!(q.quote.expires_at, now + Duration::seconds(TTL_SECONDS));
    assert_eq!(
        reservation_rows(&desk, q.quote.id).await,
        vec!["RESERVED|-|100|true".to_owned()],
        "D-4, D-15"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_a_rejected_request_refuses_the_retry() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let request_id = pending(&desk).await;
    desk.decide(
        &request_id,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Rejected,
        "not today",
    )
    .await
    .expect("the branch's manager decides");

    let mut retry = request(Side::Sell, "100");
    retry.approval_request_id = Some(request_id.as_str().to_owned());
    let got = issue(&desk, retry, whole_second_now()).await;

    assert_eq!(refusal(got), APPROVAL_REJECTED);
    assert_eq!(quotes(&desk).await, 0);
    assert_eq!(reservations(&desk).await, 0);
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_the_proposer_cannot_decide_its_own_request() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let request_id = pending(&desk).await;

    let got = desk
        .decide(
            &request_id,
            &teller().access_actor_at(
                ORG_1,
                &xc_kernel::ids::BranchId::new("MAIN"),
                whole_second_now(),
            ),
            Outcome::Approved,
            "mine",
        )
        .await;

    assert!(got.is_err(), "no self-decision: {got:?}");
    assert_eq!(
        desk.db
            .text(&format!(
                "SELECT status FROM approval_requests WHERE id = '{}'",
                request_id.as_str()
            ))
            .await
            .unwrap(),
        "PENDING"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_a_retry_that_changes_the_quantity_is_stale() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let request_id = pending(&desk).await;
    desk.decide(
        &request_id,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Approved,
        "known customer",
    )
    .await
    .expect("the branch's manager decides");

    let mut retry = request(Side::Sell, "120");
    retry.approval_request_id = Some(request_id.as_str().to_owned());
    let got = issue(&desk, retry, whole_second_now()).await;

    assert_eq!(
        refusal(got),
        STALE_PROPOSAL,
        "the approval names another proposal"
    );
    assert_eq!(quotes(&desk).await, 0);
    assert_eq!(reservations(&desk).await, 0);
    desk.db.finish().await;
}

#[tokio::test]
async fn q23_no_payment_is_written_and_an_unused_lock_is_released_by_the_clock() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();

    let q = issued(issue(&desk, request(Side::Sell, "100"), now).await);

    assert_eq!(
        desk.db
            .count("SELECT count(*) FROM transaction_payments")
            .await,
        0,
        "D-9: no payment row"
    );
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 0);
    assert_eq!(
        reserved_at(&desk, later(now, TTL_SECONDS - 1)).await,
        Decimal::from(100),
        "held until expires_at"
    );
    assert_eq!(
        reserved_at(&desk, later(now, TTL_SECONDS)).await,
        Decimal::ZERO,
        "B05 D-3: released by the clock at expires_at"
    );
    assert_eq!(quote_row(&desk, q.quote.id).await, "BINDING|ACTIVE|DESK");
    desk.db.finish().await;
}

#[tokio::test]
async fn q25_no_customer_or_one_not_verified_after_now_is_refused_on_customer() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();

    // no customer
    let mut req = request(Side::Sell, "100");
    req.quote.customer_id = None;
    assert_eq!(
        refusal(issue(&desk, req, now).await),
        NOT_VERIFIED,
        "no customer"
    );

    // Readiness None (no case), Pending, NeedsUpdate, Expired, Verified { until ≤ now }
    let cases: [(&str, Option<Case>); 6] = [
        ("CU-NONE", None),
        ("CU-PENDING", Some(("SUBMITTED", None))),
        ("CU-REVIEW", Some(("UNDER_REVIEW", None))),
        ("CU-UPDATE", Some(("NEEDS_UPDATE", None))),
        (
            "CU-EXPIRED",
            Some(("EXPIRED", Some(now - Duration::days(1)))),
        ),
        ("CU-LAPSED", Some(("VERIFIED", Some(now)))),
    ];
    for (id, case) in cases {
        customer(&desk, id).await;
        if let Some((status, until)) = case {
            verify(&desk, id, status, until).await;
        }
        let mut req = request(Side::Sell, "100");
        req.quote.customer_id = Some(id.to_owned());
        assert_eq!(
            refusal(issue(&desk, req, now).await),
            NOT_VERIFIED,
            "D-17: {id}"
        );
    }
    assert_eq!(quotes(&desk).await, 0, "no quote");
    assert_eq!(reservations(&desk).await, 0, "no reservation");
    desk.db.finish().await;
}

#[tokio::test]
async fn q26_a_customer_verified_until_after_now_is_issued() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let now = whole_second_now();
    customer(&desk, "CU-SOON").await;
    verify(
        &desk,
        "CU-SOON",
        "VERIFIED",
        Some(now + Duration::seconds(1)),
    )
    .await;

    let mut req = request(Side::Sell, "100");
    req.quote.customer_id = Some("CU-SOON".to_owned());
    let q = issued(issue(&desk, req, now).await);

    assert_eq!(q.quote.customer_id.as_deref(), Some("CU-SOON"));
    assert_eq!(quote_row(&desk, q.quote.id).await, "BINDING|ACTIVE|DESK");
    desk.db.finish().await;
}
