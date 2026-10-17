//! An approval request belongs to the organisation of its branch: a decider of another organisation never decides
//! it, whatever role it holds. Design `docs/design/blueprint/reversal.md:178-179` (FINANCE "for the reversal's
//! branch … `Authorizer` scope rules") and `org-roles-branches.md` (a role's scope is inside its organisation).
//!
//! Written before the fix by the ticket author (claude/9a464ffe). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! Drives `ApprovalService::decide` through a reversal (RV-2), the same harness as `reversal_request_test.rs`
//! (copied, since that file says not to edit it). `ORG_2` is a second organisation with its own branch `B9`
//! (`quote_common::TestDb::second_organisation`). Needs `TEST_DATABASE_URL`; nothing here skips.

mod quote_common;

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppResult;
use quote_common::{ORG_1, TestDb};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::reversal::reversal::Reversal;
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
                "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
                    customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
                    recorded_at, recorded_by, status, completed_at) \
                 VALUES ('{id}', {org}, {number}, '{branch}', '{mode}', {shift}, 'sub-{id}', 'h', '{customer}', \
                    'Somchai', 'THA', 'EMP-001', 'WALK-IN', '', '2026-10-06', '2026-10-06T03:00:00Z', \
                    '2026-10-06T03:00:00Z', 'owner', '{status}', {completed})"
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

    async fn finish(self) {
        self.db.finish().await;
    }
}

const ORG_2: &str = "00000000-0000-0000-0000-000000000002";

/// A user of `org` holding each `(role, branch)` (organisation-wide when `branch` is None).
fn member(id: &str, org: &str, roles: &[(access::Role, Option<&str>)]) -> access::Actor {
    access::Actor::new(
        id,
        org,
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

/// A shop whose MAIN (ORG_1) holds one REQUESTED reversal, beside ORG_2 and its branch B9.
async fn requested() -> (Shop, Reversal) {
    let shop = Shop::start().await;
    shop.db.second_organisation(ORG_2, "ORG2", "B9").await;
    let original = shop.exchange("MAIN", true, "COMPLETED", "3210.00").await;
    let reversal = shop
        .request("MAIN", original, "wrong currency", "k-a", inside())
        .await
        .expect("MAIN's teller requests the reversal");
    (shop, reversal)
}

async fn decisions(shop: &Shop) -> i64 {
    shop.db
        .count("SELECT count(*) FROM approval_decisions")
        .await
}

/// Bug row: an organisation-wide FINANCE of ORG_2 is refused, and nothing is written.
#[tokio::test]
async fn finance_of_another_organisation_does_not_decide() {
    let (shop, reversal) = requested().await;
    for outcome in [Outcome::Approved, Outcome::Rejected] {
        assert_eq!(
            shop.decide(
                reversal.approval_request_id,
                &member("finance-org2", ORG_2, &[(access::Role::Finance, None)]),
                outcome,
            )
            .await,
            Err("outOfScope"),
            "FINANCE of ORG_2 decides nothing of ORG_1 ({outcome:?})"
        );
    }
    assert_eq!(shop.status_of(&reversal).await, "REQUESTED");
    assert_eq!(decisions(&shop).await, 0, "no decision row");
    shop.finish().await;
}

/// Bug row: holding every role organisation-wide in ORG_2 changes nothing.
#[tokio::test]
async fn every_role_of_another_organisation_is_refused() {
    let (shop, reversal) = requested().await;
    let all = [
        (access::Role::Finance, None),
        (access::Role::Executive, None),
        (access::Role::Admin, None),
        (access::Role::BranchManager, None),
    ];
    assert_eq!(
        shop.decide(
            reversal.approval_request_id,
            &member("everyone-org2", ORG_2, &all),
            Outcome::Approved,
        )
        .await,
        Err("outOfScope")
    );
    assert_eq!(shop.status_of(&reversal).await, "REQUESTED");
    shop.finish().await;
}

/// Kept row: FINANCE of ORG_2's own branch B9 is refused (it never covered MAIN).
#[tokio::test]
async fn finance_of_another_organisations_branch_is_refused() {
    let (shop, reversal) = requested().await;
    assert_eq!(
        shop.decide(
            reversal.approval_request_id,
            &member("finance-b9", ORG_2, &[(access::Role::Finance, Some("B9"))]),
            Outcome::Approved,
        )
        .await,
        Err("outOfScope")
    );
    assert_eq!(shop.status_of(&reversal).await, "REQUESTED");
    shop.finish().await;
}

/// Kept row (#1920): an organisation-wide FINANCE of ORG_1 still decides MAIN's reversal.
#[tokio::test]
async fn finance_of_the_same_organisation_still_decides() {
    let (shop, reversal) = requested().await;
    assert_eq!(
        shop.decide(
            reversal.approval_request_id,
            &member("finance-org1", ORG_1, &[(access::Role::Finance, None)]),
            Outcome::Approved,
        )
        .await,
        Ok(())
    );
    assert_eq!(shop.status_of(&reversal).await, "APPROVED");
    assert_eq!(decisions(&shop).await, 1);
    shop.finish().await;
}

/// Kept row: FINANCE of MAIN itself still decides.
#[tokio::test]
async fn finance_of_the_branch_still_decides() {
    let (shop, reversal) = requested().await;
    assert_eq!(
        shop.decide(
            reversal.approval_request_id,
            &finance("MAIN"),
            Outcome::Rejected
        )
        .await,
        Ok(())
    );
    assert_eq!(shop.status_of(&reversal).await, "REJECTED");
    shop.finish().await;
}
