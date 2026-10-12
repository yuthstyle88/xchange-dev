//! Ticket #672 (TX-4) — `ExchangeTransactionSubject` + `ExchangeApprovalPolicy` through AP-1's
//! `ApprovalService::decide` against a real PostgreSQL: who may decide an exchange waiting for
//! approval, and what each decision does to it.
//! Written before the implementation (claude/14cc24bd, on the owner's order, 2026-10-10). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §0 T2 (*"เกินวงเงิน ผู้จัดการอนุมัติ"*),
//! §4.1 (PENDING_APPROVAL → CONFIRMED on approve, → CANCELLED with the reason on reject), §5.1
//! (`type_name() = "EXCHANGE_TRANSACTION"`; `version_hash` over the lines, the payment and the
//! customer; `on_decided`; `may_decide` ⇔ BRANCH_MANAGER **of the transaction's branch**, the
//! EXECUTIVE does not decide here; `allows_self_decision = false`). Ticket #672 G4: the HTTP
//! route `POST /api/approvals/:id/decide` does not admit a branch manager on main, so these tests
//! decide through `ApprovalService::decide`, the service that route calls; the route's own test
//! waits for G4's ticket.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Every call is committed whatever it answers, and the rows are read back.
//!
//! Signatures this test assumes ([required] in ticket §7 unless marked "choice"):
//! - TX-4 registers `EXCHANGE_TRANSACTION` with AP-1 in
//!   `presentation::http::v3::commands::make_approval_service(pool)`, beside RATE_PUBLICATION
//!   (the one-line registration of ticket §5).
//! - `may_decide` refuses with the code `outOfScope` (`Refusal::new("outOfScope", …)`), which
//!   `ApprovalError::code` answers as `outOfScope` (§9: *"The teller cannot decide (`outOfScope`),
//!   nor can a manager of another branch"*).
//! - `Role::Manager` is the Blueprint's BRANCH_MANAGER; `Actor::owner` is the EXECUTIVE.
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `decide` takes
//! `access::Actor`, so every decider is the B1-mapped user (`tx4_common::staff`/`owner`). One outcome changes,
//! named by ruling 1 on #1020: BACKEND and MANAGER are both BRANCH_MANAGER, so BACKEND now decides.
//! `no_other_role_of_the_branch_decides` therefore leaves out every role §3.3 makes a BRANCH_MANAGER, and
//! `a_backend_employee_decides_as_the_branchs_manager` pins the change. Every other assertion is unchanged.
//! - `on_decided` receives no reason (`xc-approval/src/domain/subject.rs`); the REJECTED reason it
//!   stores is the reason of the decision row `decide` has written in the same transaction.

mod quote_common;
mod tx4_common;

use quote_common::*;
use tx4_common::*;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionStatus};
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;

const MANAGER: &str = "employee-manager";

/// A MAIN exchange waiting for approval, and its request.
async fn pending(desk: &Desk) -> (ExchangeTransaction, RequestId) {
    let now = whole_second_now();
    let (_, t) = desk.main_draft(now).await;
    desk.limit("MAIN", "1000.00").await;
    match desk.confirm(t.id, later(now, 10)).await {
        Ok(ConfirmOutcome::PendingApproval(request)) => (t, request),
        other => panic!("3210.00 > 1000.00 waits for approval: {other:?}"),
    }
}

async fn request_status(desk: &Desk, request: &RequestId) -> String {
    desk.db
        .text(&format!(
            "SELECT status FROM approval_requests WHERE id = '{}'",
            request.as_str()
        ))
        .await
        .expect("a request row")
}

#[tokio::test]
async fn the_branchs_manager_approves_and_the_exchange_is_confirmed() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;

    desk.decide(
        &request,
        &staff(MANAGER, Role::Manager, "MAIN"),
        Outcome::Approved,
        "known customer",
    )
    .await
    .expect("T2: the branch's manager approves");

    let stored = desk.stored(t.id).await;
    assert_eq!(
        stored.status,
        TransactionStatus::Confirmed,
        "§4.1 approve ⇒ CONFIRMED"
    );
    assert_eq!(
        stored.approval_request_id.map(|u| u.to_string()),
        Some(request.as_str().to_owned()),
        "the exchange still names the request it was approved by"
    );
    assert_eq!(stored.cancel_reason, None);
    assert_eq!(request_status(&desk, &request).await, "APPROVED");
    desk.db.finish().await;
}

#[tokio::test]
async fn the_branchs_manager_rejects_and_the_exchange_is_cancelled_with_the_reason() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;

    desk.decide(
        &request,
        &staff(MANAGER, Role::Manager, "MAIN"),
        Outcome::Rejected,
        "passport photo unclear",
    )
    .await
    .expect("the branch's manager rejects");

    let stored = desk.stored(t.id).await;
    assert_eq!(
        stored.status,
        TransactionStatus::Cancelled,
        "§4.1 reject ⇒ CANCELLED"
    );
    assert_eq!(
        stored.cancel_reason.as_deref(),
        Some("passport photo unclear"),
        "§5.1: with the decision's reason"
    );
    assert!(stored.cancelled_at.is_some());
    assert_eq!(stored.document_ref, None);
    assert_eq!(request_status(&desk, &request).await, "REJECTED");
    desk.db.finish().await;
}

#[tokio::test]
async fn a_teller_of_the_branch_cannot_decide() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;

    assert_eq!(
        approval_code(
            desk.decide(
                &request,
                &staff("employee-other-teller", Role::Front, "MAIN"),
                Outcome::Approved,
                "ok",
            )
            .await
        ),
        "outOfScope",
        "§9: the teller cannot decide (not the proposer: another teller of MAIN)"
    );
    assert_eq!(
        desk.stored(t.id).await.status,
        TransactionStatus::PendingApproval
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");
    desk.db.finish().await;
}

#[tokio::test]
async fn a_manager_of_another_branch_cannot_decide() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;

    for outcome in [Outcome::Approved, Outcome::Rejected] {
        assert_eq!(
            approval_code(
                desk.decide(
                    &request,
                    &staff("employee-br2-manager", Role::Manager, BR2),
                    outcome,
                    "not mine",
                )
                .await
            ),
            "outOfScope",
            "§5.1: BRANCH_MANAGER of the transaction's branch only"
        );
    }
    assert_eq!(
        desk.stored(t.id).await.status,
        TransactionStatus::PendingApproval
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");
    desk.db.finish().await;
}

/// §5.1: *"EXECUTIVE is not named by T2 and does not decide here."*
#[tokio::test]
async fn the_owner_does_not_decide_an_exchange() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;

    assert_eq!(
        approval_code(
            desk.decide(&request, &owner("owner"), Outcome::Approved, "ok")
                .await
        ),
        "outOfScope"
    );
    assert_eq!(
        desk.stored(t.id).await.status,
        TransactionStatus::PendingApproval
    );
    desk.db.finish().await;
}

/// Other staff roles of the branch are not its manager either.
#[tokio::test]
async fn no_other_role_of_the_branch_decides() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;

    // OR-7.1 #1020 ruling 1: BACKEND is a BRANCH_MANAGER too (§3.3).
    for role in Role::STAFF
        .into_iter()
        .filter(|r| !matches!(r, Role::Manager | Role::Backend))
    {
        assert_eq!(
            approval_code(
                desk.decide(
                    &request,
                    &staff(&format!("employee-{}", role.as_str()), role, "MAIN"),
                    Outcome::Approved,
                    "ok",
                )
                .await
            ),
            "outOfScope",
            "{role:?}"
        );
    }
    assert_eq!(
        desk.stored(t.id).await.status,
        TransactionStatus::PendingApproval
    );
    desk.db.finish().await;
}

/// OR-7.1 #1020 ruling 1 (named change): §3.3 maps BACKEND to BRANCH_MANAGER, the role that decides an exchange of
/// its branch, so a BACKEND employee of the branch now approves it (before #1020 only MANAGER did).
#[tokio::test]
async fn a_backend_employee_decides_as_the_branchs_manager() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;

    desk.decide(
        &request,
        &staff("employee-BACKEND", Role::Backend, "MAIN"),
        Outcome::Approved,
        "known customer",
    )
    .await
    .expect("BACKEND, a BRANCH_MANAGER of MAIN, approves");
    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Confirmed);
    assert_eq!(request_status(&desk, &request).await, "APPROVED");
    desk.db.finish().await;
}

/// §5.1 `version_hash` over the lines, the payment and the customer: a line changed after the
/// request makes the decision stale (AP-1 answers `staleVersion`), and nothing moves.
#[tokio::test]
async fn a_line_changed_after_the_request_makes_the_decision_stale() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;
    desk.db
        .exec(&format!(
            "UPDATE transaction_lines SET quantity = quantity + 1 WHERE transaction_id = '{}'",
            t.id.0
        ))
        .await;

    assert_eq!(
        approval_code(
            desk.decide(
                &request,
                &staff(MANAGER, Role::Manager, "MAIN"),
                Outcome::Approved,
                "ok",
            )
            .await
        ),
        "staleVersion"
    );
    assert_eq!(
        desk.stored(t.id).await.status,
        TransactionStatus::PendingApproval
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");
    desk.db.finish().await;
}

#[tokio::test]
async fn a_payment_changed_after_the_request_makes_the_decision_stale() {
    let desk = Desk::start().await;
    let (t, request) = pending(&desk).await;
    desk.db
        .exec(&format!(
            "UPDATE transaction_payments SET amount_thb = amount_thb + 1 WHERE transaction_id = '{}'",
            t.id.0
        ))
        .await;

    assert_eq!(
        approval_code(
            desk.decide(
                &request,
                &staff(MANAGER, Role::Manager, "MAIN"),
                Outcome::Approved,
                "ok",
            )
            .await
        ),
        "staleVersion"
    );
    assert_eq!(
        desk.stored(t.id).await.status,
        TransactionStatus::PendingApproval
    );
    desk.db.finish().await;
}
