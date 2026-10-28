//! Up-front tests for BQ-8 (design `docs/design/blueprint/binding-quote.md` D-20, §4.2 Q31, Q32): an APPROVED
//! `BINDING_QUOTE` request issues **one** binding quote. Written by the sub-controller claude/644b3958 on
//! `feat/binding-quote` before the implementation (lane controller's order on PR #5281). **Do not edit, delete or
//! weaken.**
//!
//! `IssueBindingQuote::run_in` records the APPROVED request it issued under in `quotes.approval_request_id` (UNIQUE,
//! BQ-0's migration), in the issue's transaction. A later issue naming the same request is refused
//! `APPROVAL_USED` before anything is written, whether or not the first quote is still ACTIVE. A replay with the same
//! `Idempotency-Key` is the command seam's and keeps its first answer (`v3_binding_quote_issue_test::q6_…`).
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod binding_issue_common;
mod quote_common;
mod tx4_common;

use binding_issue_common::*;
use quote_common::*;
use tx4_common::*;
use uuid::Uuid;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::binding_issue::APPROVAL_USED;

const MANAGER: &str = "employee-manager";

/// A binding quote of 100 USD over MAIN's 1000.00 limit: its PENDING request, then APPROVED by MAIN's manager.
async fn approved(desk: &Desk) -> RequestId {
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
    id
}

fn naming(
    id: &RequestId,
) -> xchange::application::workflows::exchange::binding_issue::BindingIssueRequest {
    let mut r = request(Side::Sell, "100");
    r.approval_request_id = Some(id.as_str().to_owned());
    r
}

async fn recorded_request(desk: &Desk, quote_id: Uuid) -> Option<String> {
    desk.db
        .text(&format!(
            "SELECT approval_request_id::text FROM quotes WHERE id = '{quote_id}'"
        ))
        .await
}

#[tokio::test]
async fn q31_the_quote_records_the_approved_request_it_was_issued_under() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = approved(&desk).await;

    let q = issued(issue(&desk, naming(&id), whole_second_now()).await);

    assert_eq!(
        recorded_request(&desk, q.quote.id).await.as_deref(),
        Some(id.as_str()),
        "D-20: quotes.approval_request_id names the request"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q31_a_quote_issued_with_no_approval_records_none() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let q = issued(issue(&desk, request(Side::Sell, "10"), whole_second_now()).await);
    assert_eq!(recorded_request(&desk, q.quote.id).await, None);
    desk.db.finish().await;
}

#[tokio::test]
async fn q32_a_second_issue_naming_the_same_approved_request_is_refused_and_writes_nothing() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = approved(&desk).await;
    let first = issued(issue(&desk, naming(&id), whole_second_now()).await);
    let (quotes_before, reservations_before) = (quotes(&desk).await, reservations(&desk).await);

    let again = issue(&desk, naming(&id), whole_second_now()).await;

    assert_eq!(refusal(again), APPROVAL_USED);
    assert_eq!(quotes(&desk).await, quotes_before, "no second quote");
    assert_eq!(
        reservations(&desk).await,
        reservations_before,
        "no second reservation"
    );
    assert_eq!(
        quote_row(&desk, first.quote.id).await,
        "BINDING|ACTIVE|DESK"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q32_the_request_stays_used_after_its_quote_is_cancelled() {
    let desk = desk("300", Some(TTL_SECONDS)).await;
    let id = approved(&desk).await;
    let first = issued(issue(&desk, naming(&id), whole_second_now()).await);
    desk.db
        .exec(&format!(
            "UPDATE quotes SET status = 'CANCELLED' WHERE id = '{}'",
            first.quote.id
        ))
        .await;

    let again = issue(&desk, naming(&id), whole_second_now()).await;

    assert_eq!(refusal(again), APPROVAL_USED);
    desk.db.finish().await;
}
