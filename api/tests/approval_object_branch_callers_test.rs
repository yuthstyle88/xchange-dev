//! #5279 site 2 through a real caller (design `docs/design/org-wide-actor-branch.md` §3 row 7, §5 row S2-7; owner,
//! #5279 issuecomment-6452390376: *"เก็บที่สาขาของสิ่งที่ขอ"*). Written before the fix by the sub-controller
//! claude/2ca5874b. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The owner confirms an exchange of `BR2` over `BR2`'s limit through `ConfirmTransaction`, the path that calls
//! `ApprovalService::request` (`confirm.rs:151`). The request belongs to `BR2`: `BR2`'s manager decides it, and
//! `MAIN`'s does not. Before #5279 `requester_branch` filed an owner's request with no branch, so only an
//! organisation-wide row could cover it (`ScopedRole::covers`), and `BR2`'s manager was refused.
//!
//! Lane controller's follow-up on PR #5460: the test must not pass for the wrong reason. So it also pins the A5
//! outcome: `ExchangeApprovalPolicy` keeps the trait's `allows_self_decision = false`
//! (`xc-approval/src/domain/policy.rs:38-41`), so the owner who asked cannot close the request themselves, and it
//! is still PENDING when `BR2`'s manager approves it.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod quote_common;
mod tx4_common;

use quote_common::*;
use tx4_common::*;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_auth::domain::access::StandingKind;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::confirm::{ConfirmOutcome, ConfirmTransaction};
use xchange::application::workflows::save_step::{SaveActor, SaveExtras};

const OWNER: &str = "owner-5279";

/// The owner's saver: what `ConfirmTransaction` turns into the requester (`SaveActor::access_actor_at`).
fn owner_saver() -> SaveActor {
    SaveActor {
        id: OWNER.to_owned(),
        username: "owner".to_owned(),
        employee_id: None,
        standing: StandingKind::Owner,
    }
}

/// `BR2`'s exchange, confirmed by the owner over `BR2`'s limit: the request it opened.
async fn pending_at_br2(desk: &Desk) -> RequestId {
    let now = whole_second_now();
    let q = desk.quote(BR2, Side::Buy, "100", None, now).await;
    let t = desk.draft(&q, now).await;
    desk.limit(BR2, "1000.00").await;
    let service = ConfirmTransaction::new(desk.db.pool.clone(), desk.approvals.clone(), Vec::new());
    let mut tx = desk.db.pool.begin().await.unwrap();
    let got = service
        .run_in(
            &mut tx,
            t.id,
            &owner_saver(),
            &SaveExtras::default(),
            later(now, 10),
        )
        .await;
    tx.commit().await.unwrap();
    match got {
        Ok(ConfirmOutcome::PendingApproval(request)) => request,
        other => panic!("3210.00 > 1000.00 at BR2 waits for approval: {other:?}"),
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

/// S2-7: the owner's over-limit exchange at `BR2` is filed at `BR2`, and `BR2`'s manager decides it.
#[tokio::test]
async fn the_owners_over_limit_exchange_is_filed_at_its_branch_and_its_manager_decides() {
    let desk = Desk::start().await;
    let request = pending_at_br2(&desk).await;

    let row = desk.request_row(&request).await;
    let fields: Vec<&str> = row.split('|').collect();
    assert_eq!(
        (fields[0], fields[2], fields[3]),
        ("EXCHANGE_TRANSACTION", BR2, OWNER),
        "the request of BR2's exchange belongs to BR2, whoever asked: {row}"
    );

    // A5: the policy does not let the proposer close it (`allows_self_decision = false`).
    let mut tx = desk.db.pool.begin().await.unwrap();
    let own = desk
        .approvals
        .decide_self(&mut tx, &request, &owner(OWNER), "mine")
        .await;
    tx.rollback().await.unwrap();
    assert_eq!(
        approval_code(own),
        "outOfScope",
        "A5: the owner who asked does not decide it"
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");

    // Another branch's manager does not cover it.
    assert_eq!(
        approval_code(
            desk.decide(
                &request,
                &staff_as("employee-main-manager", "MANAGER", "MAIN"),
                Outcome::Approved,
                "not mine",
            )
            .await
        ),
        "outOfScope",
        "MAIN's manager does not decide BR2's exchange"
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");

    // BR2's manager does. Before #5279 the request had no branch, and this was refused.
    desk.decide(
        &request,
        &staff_as("employee-br2-manager", "MANAGER", BR2),
        Outcome::Approved,
        "checked",
    )
    .await
    .unwrap_or_else(|e| panic!("BR2's manager decides BR2's exchange: {}", e.code()));
    assert_eq!(request_status(&desk, &request).await, "APPROVED");
    desk.db.finish().await;
}
