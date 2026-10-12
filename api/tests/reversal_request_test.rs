//! Ticket #682 (RV-1+RV-2), Part 2 (RV-2) — requesting a reversal and FINANCE's decision, at the
//! service level against a real PostgreSQL.
//! Written before the implementation by the ticket author for the TX/RV chain (claude/f03345fe, on
//! the lane controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/reversal.md`:
//! - §2: V-1, whole transaction, `amount_thb` = Σ the original's `settle_thb`; V-2, the original is
//!   never changed; V-5, one live reversal;
//! - §4.2: COMPLETED, LIVE, of this branch, else `outOfScope`/`staleVersion`; inside the window,
//!   else `reversalWindowClosed`; no live reversal, else `staleVersion` naming the existing one;
//!   insert REQUESTED and `ApprovalService::request(tx, "TRANSACTION_REVERSAL", id, …)`;
//! - §5: `on_decided` APPROVED ⇒ APPROVED, REJECTED ⇒ REJECTED; `may_decide` ⇔ FINANCE of the
//!   reversal's branch; the requester never decides.
//!
//! The route and its gate (`(Rpc, "reversalRequest")`, ticket #874) are pinned in
//! `api/tests/v3_reversal_request_route_test.rs`. This file drives the workflow and AP-1 directly,
//! so it does not wait for #874.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Rows are seeded with SQL in TX-2's schema (#670).
//! The original was completed at 2026-10-06 03:00Z (10:00 in Bangkok, MAIN's timezone by OR-1's
//! default); "inside the window" is 05:00Z the same day, "after it" is 17:00Z (00:00 the next day in
//! Bangkok).
//!
//! Signatures this test assumes ([required] in ticket §7 unless marked; RG5 decided by the ticket
//! author, 2026-10-10: the design's `run_in` has no branch, yet must refuse another branch's
//! transaction, so the branch travels in the input):
//! - `xchange::application::workflows::exchange::reversal_request::{RequestReversal,
//!   RequestReversalInput}`:
//!   - `RequestReversal::new(pool: PgPool, approvals: Arc<ApprovalService>)`;
//!   - `RequestReversal::run_in(&self, tx, input: RequestReversalInput, who: &SaveActor, now:
//!     DateTime<Utc>) -> AppResult<Reversal>` (async);
//!   - `RequestReversalInput { branch: BranchId, original: TransactionId, reason: String, key:
//!     String }`, every field public;
//! - `xchange::application::workflows::exchange::reversal_approval::register(registry: &mut
//!   ApprovalRegistry, pool: PgPool)`, as `rate_publication::register`;
//! - `xc_exchange::domain::reversal::reversal::{Reversal, ReversalId, ReversalStatus}`:
//!   `ReversalId(pub Uuid)`; `Reversal` has public `id`, `status`, `approval_request_id: Uuid`;
//! - TX-2's `TransactionId(pub Uuid)` (#670);
//! - a refusal is `Err(AppError::Domain(m))` with `m` starting with its code (`outOfScope`,
//!   `staleVersion`, `reversalWindowClosed`, `validationFailed`), as TX-1's `QuoteService` answers.
//!   For a second live reversal, `m` also contains the existing reversal's id. An unknown
//!   transaction is `outOfScope`, like another branch's (ticket author's choice: no existence oracle);
//! - `ReversalPolicy::may_decide` refuses with `Refusal::new("outOfScope", …)`, as
//!   `RatePublicationPolicy` does, so AP-1 answers `outOfScope`;
//! - AP-1's own codes (`ApprovalError::code()`) for the decision.
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `ApprovalService::decide`
//! takes `access::Actor`, so the deciders are the B1-mapped users (`user`): FINANCE, BRANCH_MANAGER and TELLER
//! on their branch, the owner EXECUTIVE + ADMIN organisation-wide. The requester's `SaveActor` is unchanged.
//! Every assertion is unchanged.

mod quote_common;

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use quote_common::{ORG_1, TestDb};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::reversal::reversal::{Reversal, ReversalStatus};
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_kernel::ids::BranchId;
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use xchange::application::workflows::save_step::SaveActor;

fn inside() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 5, 0, 0).unwrap()
}

fn after() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 17, 0, 0).unwrap()
}

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-front".to_owned(),
        username: "front".to_owned(),
        employee_id: Some("employee-front".to_owned()),
        role: Role::Front,
    }
}

/// A user holding each `(role, branch)` (organisation-wide when `branch` is None), the B1 mapping of a v3 actor.
fn user(id: &str, roles: &[(access::Role, Option<&str>)]) -> access::Actor {
    access::Actor::new(
        id,
        "00000000-0000-0000-0000-000000000001",
        roles
            .iter()
            .map(|(role, branch)| {
                access::ScopedRole::new(
                    *role,
                    branch.map(BranchId::new),
                    chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                    None,
                )
            })
            .collect(),
        None,
    )
}

fn finance(branch: &str) -> access::Actor {
    user(
        &format!("finance-{branch}"),
        &[(access::Role::Finance, Some(branch))],
    )
}

struct Shop {
    db: TestDb,
    approvals: Arc<ApprovalService>,
    requests: RequestReversal,
}

impl Shop {
    async fn start() -> Self {
        let db = TestDb::start().await;
        db.branch("B2").await;
        for branch in ["MAIN", "B2"] {
            db.exec(&format!(
                "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                 SELECT c.id, b.organization_id, c.id, 'Somchai', c.mode, now() FROM branches b, \
                    (VALUES ('CU-{branch}', 'LIVE'), ('CD-{branch}', 'DEMO')) AS c (id, mode) \
                 WHERE b.id = '{branch}'"
            ))
            .await;
            // a teller user owns the shift (CA-1's `shifts.teller_id`, `20261012100000_cash.sql`)
            db.exec(&format!(
                "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                     SELECT organization_id, 'Teller {branch}', 'ACTIVE', 'test' FROM branches \
                     WHERE id = '{branch}' RETURNING id) \
                 INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, \
                     teller_id) \
                 SELECT 'SH-{branch}', '{branch}', 'OPEN', now(), 'owner', 'open-{branch}', u.id FROM u"
            ))
            .await;
        }
        let mut registry = ApprovalRegistry::new();
        reversal_approval::register(&mut registry, db.pool.clone());
        let approvals = Arc::new(ApprovalService::new(
            registry,
            Arc::new(PgApprovalRepository::new(db.pool.clone())),
            Arc::new(PgAuditLog::new(db.pool.clone())),
        ));
        let requests = RequestReversal::new(db.pool.clone(), approvals.clone());
        Self {
            db,
            approvals,
            requests,
        }
    }

    /// A LIVE (or DEMO) exchange at `branch` in `status`, with one line settling `settle` (THB amount
    /// `settle + 10`, fee 10, so the settled THB differs from the principal); answers
    /// its id. Completed ones carry a number and `completed_at` 2026-10-06 03:00Z.
    async fn exchange(
        &self,
        branch: &str,
        live: bool,
        status: &str,
        settle: &str,
    ) -> TransactionId {
        let id = uuid::Uuid::new_v4();
        let n = self.db.count("SELECT count(*) FROM transactions").await + 1;
        let (mode, shift, customer) = if live {
            ("LIVE", format!("'SH-{branch}'"), format!("CU-{branch}"))
        } else {
            ("DEMO", "NULL".to_owned(), format!("CD-{branch}"))
        };
        let (number, completed) = if status == "COMPLETED" {
            (
                format!("'XC-20261006-{n:06}'"),
                "'2026-10-06T03:00:00Z'".to_owned(),
            )
        } else {
            ("NULL".to_owned(), "NULL".to_owned())
        };
        let org = format!("(SELECT organization_id FROM branches WHERE id = '{branch}')");
        self.db
            .exec(&format!(
                "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, \
                    submission_id, payload_hash, customer_id, customer_name, nationality, type, code, \
                    amount, rate, principal, fee, bank_fee, received, paid, method, payment_ref, \
                    account, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
                    recorded_by, status, completed_at) \
                 VALUES ('{id}', {org}, {number}, '{branch}', '{mode}', {shift}, 'sub-{id}', 'h', \
                    '{customer}', 'Somchai', 'THA', 'BUY', 'USD', 100, 32.1, {settle} + 10, 10, 0, 0, \
                    {settle}, 'Cash', '', 'Cash drawer 1', 'EMP-001', 'WALK-IN', '', '2026-10-06', \
                    '2026-10-06T03:00:00Z', '2026-10-06T03:00:00Z', 'owner', '{status}', {completed})"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                    quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
                    created_at, updated_at) \
                 VALUES (gen_random_uuid(), {org}, '{id}', 'BUY', 'USD', 100, 1, 32.1, {settle} + 10, 10, \
                    {settle}, 1, now(), now())"
            ))
            .await;
        TransactionId(id)
    }

    async fn request(
        &self,
        branch: &str,
        original: TransactionId,
        reason: &str,
        key: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Reversal> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .requests
            .run_in(
                &mut tx,
                RequestReversalInput {
                    branch: BranchId::new(branch),
                    original,
                    reason: reason.to_owned(),
                    key: key.to_owned(),
                },
                &teller(),
                now,
            )
            .await;
        // committed even when refused: a refusal must have written nothing
        tx.commit().await.unwrap();
        got
    }

    async fn decide(
        &self,
        request: uuid::Uuid,
        who: &access::Actor,
        outcome: Outcome,
    ) -> Result<(), &'static str> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .approvals
            .decide(
                &mut tx,
                &RequestId::new(request.to_string()),
                who,
                outcome,
                "checked the drawer",
                &format!("decide-{}", uuid::Uuid::new_v4()),
            )
            .await;
        match got {
            Ok(()) => {
                tx.commit().await.unwrap();
                Ok(())
            }
            Err(e) => {
                tx.rollback().await.unwrap();
                Err(e.code())
            }
        }
    }

    async fn status_of(&self, reversal: &Reversal) -> String {
        self.db
            .text(&format!(
                "SELECT status FROM reversals WHERE id = '{}'",
                reversal.id.0
            ))
            .await
            .expect("a status")
    }

    /// (reversals, approval_requests of the type)
    async fn written(&self) -> (i64, i64) {
        (
            self.db.count("SELECT count(*) FROM reversals").await,
            self.db
                .count("SELECT count(*) FROM approval_requests WHERE type = 'TRANSACTION_REVERSAL'")
                .await,
        )
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

fn code<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m.split(':').next().unwrap_or("").trim().to_owned(),
        other => panic!("expected Err(AppError::Domain(\"<code>: …\")), got {other:?}"),
    }
}

#[tokio::test]
async fn a_request_stores_a_requested_reversal_of_the_whole_exchange_and_opens_an_approval() {
    let shop = Shop::start().await;
    let original = shop.exchange("MAIN", true, "COMPLETED", "3210.00").await;
    let r = shop
        .request("MAIN", original, "wrong currency", "k-1", inside())
        .await
        .expect("a reversal is requested");
    assert_eq!(r.status, ReversalStatus::Requested);
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT concat_ws('|', status, original_transaction_id::text, reason, amount_thb::text, \
                    business_date::text, requested_by, organization_id::text, branch_id, \
                    coalesce(posting_id::text, 'NULL'), coalesce(receipt_id::text, 'NULL')) \
                 FROM reversals WHERE id = '{}'",
                r.id.0
            ))
            .await
            .as_deref(),
        Some(
            format!(
                "REQUESTED|{}|wrong currency|3210.00|2026-10-06|employee-front|{ORG_1}|MAIN|NULL|NULL",
                original.0
            )
            .as_str()
        ),
        "V-1: the whole settled THB; R-1: the business date in Bangkok"
    );
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT concat_ws('|', type, object_id, status, branch_id) FROM approval_requests \
                 WHERE id = '{}'",
                r.approval_request_id
            ))
            .await
            .as_deref(),
        Some(format!("TRANSACTION_REVERSAL|{}|PENDING|MAIN", r.id.0).as_str()),
        "§4.2: an AP-1 request of the type, for this reversal"
    );
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT status FROM transactions WHERE id = '{}'",
                original.0
            ))
            .await
            .as_deref(),
        Some("COMPLETED"),
        "V-2: the original is never changed"
    );
    shop.finish().await;
}

#[tokio::test]
async fn only_a_completed_live_exchange_of_this_branch_inside_its_day_may_be_reversed() {
    let shop = Shop::start().await;
    let confirmed = shop.exchange("MAIN", true, "CONFIRMED", "3210.00").await;
    let demo = shop.exchange("MAIN", false, "COMPLETED", "3210.00").await;
    let other = shop.exchange("B2", true, "COMPLETED", "3210.00").await;
    let ok = shop.exchange("MAIN", true, "COMPLETED", "3210.00").await;

    assert_eq!(
        code(shop.request("MAIN", confirmed, "x", "k-c", inside()).await),
        "staleVersion",
        "not COMPLETED"
    );
    assert_eq!(
        code(shop.request("MAIN", demo, "x", "k-d", inside()).await),
        "outOfScope",
        "DEMO"
    );
    assert_eq!(
        code(shop.request("MAIN", other, "x", "k-o", inside()).await),
        "outOfScope",
        "another branch's"
    );
    assert_eq!(
        code(
            shop.request(
                "MAIN",
                TransactionId(uuid::Uuid::new_v4()),
                "x",
                "k-n",
                inside()
            )
            .await
        ),
        "outOfScope",
        "an unknown transaction, as another branch's: no existence is told"
    );
    assert_eq!(
        code(shop.request("MAIN", ok, "x", "k-late", after()).await),
        "reversalWindowClosed",
        "R-1: the next business day in Bangkok"
    );
    assert_eq!(
        code(shop.request("MAIN", ok, "", "k-empty", inside()).await),
        "validationFailed",
        "an empty reason"
    );
    assert_eq!(
        code(
            shop.request("MAIN", ok, &"x".repeat(501), "k-long", inside())
                .await
        ),
        "validationFailed",
        "a reason over 500 characters"
    );
    assert_eq!(shop.written().await, (0, 0), "nothing written by a refusal");
    shop.finish().await;
}

#[tokio::test]
async fn a_second_live_reversal_is_refused_naming_the_first_and_a_rejected_one_allows_another() {
    let shop = Shop::start().await;
    let original = shop.exchange("MAIN", true, "COMPLETED", "3210.00").await;
    let first = shop
        .request("MAIN", original, "wrong currency", "k-1", inside())
        .await
        .unwrap();
    match shop
        .request("MAIN", original, "again", "k-2", inside())
        .await
    {
        Err(AppError::Domain(m)) => {
            assert!(m.starts_with("staleVersion"), "{m}");
            assert!(
                m.contains(&first.id.0.to_string()),
                "V-5: the refusal names the live reversal: {m}"
            );
        }
        other => panic!("a second live reversal: {other:?}"),
    }
    assert_eq!(shop.written().await, (1, 1));

    assert_eq!(
        shop.decide(
            first.approval_request_id,
            &finance("MAIN"),
            Outcome::Rejected
        )
        .await,
        Ok(())
    );
    let second = shop
        .request("MAIN", original, "the right reason", "k-3", inside())
        .await
        .expect("a REJECTED reversal is followed by a new request");
    assert_ne!(second.id, first.id);
    shop.finish().await;
}

#[tokio::test]
async fn finance_of_the_branch_decides_and_nobody_else_does() {
    let shop = Shop::start().await;
    let a = shop.exchange("MAIN", true, "COMPLETED", "3210.00").await;
    let b = shop.exchange("MAIN", true, "COMPLETED", "1000.00").await;
    let ra = shop
        .request("MAIN", a, "wrong currency", "k-a", inside())
        .await
        .unwrap();
    let rb = shop
        .request("MAIN", b, "wrong amount", "k-b", inside())
        .await
        .unwrap();

    let requester = user("employee-front", &[(access::Role::Teller, Some("MAIN"))]);
    for (who, what) in [
        (requester, "the requester (a teller)"),
        (
            user("mgr", &[(access::Role::BranchManager, Some("MAIN"))]),
            "the branch manager",
        ),
        (finance("B2"), "FINANCE of another branch"),
        (
            user(
                "owner-1",
                &[(access::Role::Executive, None), (access::Role::Admin, None)],
            ),
            "the owner (R-3 names FINANCE)",
        ),
    ] {
        assert_eq!(
            shop.decide(ra.approval_request_id, &who, Outcome::Approved)
                .await,
            Err("outOfScope"),
            "{what}"
        );
    }
    assert_eq!(shop.status_of(&ra).await, "REQUESTED", "still undecided");

    assert_eq!(
        shop.decide(ra.approval_request_id, &finance("MAIN"), Outcome::Approved)
            .await,
        Ok(())
    );
    assert_eq!(shop.status_of(&ra).await, "APPROVED", "§5: on_decided");
    assert_eq!(
        shop.decide(rb.approval_request_id, &finance("MAIN"), Outcome::Rejected)
            .await,
        Ok(())
    );
    assert_eq!(shop.status_of(&rb).await, "REJECTED");
    assert_eq!(
        shop.db
            .count(
                "SELECT count(*) FROM transactions \
                 WHERE status = 'COMPLETED' AND mode = 'LIVE' AND branch_id = 'MAIN'"
            )
            .await,
        2,
        "V-2: deciding changes neither original"
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_reason_or_amount_changed_after_the_request_makes_the_decision_stale() {
    let shop = Shop::start().await;
    let original = shop.exchange("MAIN", true, "COMPLETED", "3210.00").await;
    let r = shop
        .request("MAIN", original, "wrong currency", "k-1", inside())
        .await
        .unwrap();
    shop.db
        .exec(&format!(
            "UPDATE reversals SET reason = 'something else' WHERE id = '{}'",
            r.id.0
        ))
        .await;
    assert_eq!(
        shop.decide(r.approval_request_id, &finance("MAIN"), Outcome::Approved)
            .await,
        Err("staleVersion"),
        "§5: version_hash covers the reason"
    );
    shop.finish().await;
}

/// §5: `version_hash` covers the amount too (added by the test-writer claude/dell-claude-6 on the lane controller's
/// ruling 5, #682 issuecomment-6107713287): a decision on a reversal whose amount changed after the request is stale.
#[tokio::test]
async fn an_amount_changed_after_the_request_makes_the_decision_stale() {
    let shop = Shop::start().await;
    let original = shop.exchange("MAIN", true, "COMPLETED", "3210.00").await;
    let r = shop
        .request("MAIN", original, "wrong currency", "k-1", inside())
        .await
        .unwrap();
    shop.db
        .exec(&format!(
            "UPDATE reversals SET amount_thb = amount_thb + 1 WHERE id = '{}'",
            r.id.0
        ))
        .await;
    assert_eq!(
        shop.decide(r.approval_request_id, &finance("MAIN"), Outcome::Approved)
            .await,
        Err("staleVersion"),
        "§5: version_hash covers the amount"
    );
    assert_eq!(shop.status_of(&r).await, "REQUESTED", "nothing decided");
    shop.finish().await;
}
