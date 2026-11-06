//! `TransferApprovalSubject` and `TransferApprovalPolicy` implementing `xc_approval` traits (D:§4.5, A5).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::{ApprovalSubject, SubjectError};
use xc_auth::domain::access::{Actor, ApprovalDecider, Permission, RoleGrants};
use xc_kernel::ids::BranchId;

use crate::domain::posting::CashPostings;
use crate::domain::shortfall::{ShortPlace, StockShortfall};
use crate::domain::transfer::kinds::TransferKinds;
use crate::domain::transfer::transfer_repository::TransferRepository;
use crate::infrastructure::ledger_postings::LedgerCashPostings;
use crate::infrastructure::transfer_repository::PgTransferRepository;

/// The type name.
pub const CASH_TRANSFER: &str = "CASH_TRANSFER";

/// Registers `CASH_TRANSFER` with `registry`.
pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    registry.register(
        Arc::new(TransferApprovalSubject::new(pool)),
        Arc::new(TransferApprovalPolicy::new()),
    );
}

/// The approval core's actor for `who` acting on a transfer whose source is `source`, or `None` when no role of
/// `who` grants `permission` there. There is no fallback: an actor that maps to nothing is refused by the caller.
pub fn policy_actor(
    who: &Actor,
    source: &BranchId,
    permission: Permission,
    now: DateTime<Utc>,
) -> Option<Actor> {
    let has_perm = who.roles.iter().any(|r| {
        r.valid_from <= now
            && r.valid_to.is_none_or(|to| now < to)
            && RoleGrants::holds(r.group_code(), permission)
            && (r.branch.is_none() || r.branch.as_ref() == Some(source))
    });
    if has_perm { Some(who.clone()) } else { None }
}

#[derive(Clone)]
pub struct TransferApprovalSubject {
    _pool: PgPool,
}

impl TransferApprovalSubject {
    pub fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }
}

impl ApprovalSubject for TransferApprovalSubject {
    fn type_name(&self) -> &'static str {
        CASH_TRANSFER
    }

    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM cash_transfers WHERE id = $1::uuid")
                    .bind(object_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(pos_error::AppError::Database)?;

            match status {
                Some(s) => Ok(s),
                None => Err(pos_error::AppError::NotFound {
                    entity: "CashTransfer",
                    id: object_id.to_string(),
                }),
            }
        })
    }

    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            match outcome {
                Outcome::Approved => {
                    sqlx::query(
                        "UPDATE cash_transfers SET status = 'APPROVED', updated_at = now() WHERE id = $1::uuid",
                    )
                    .bind(object_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(pos_error::AppError::Database)?;
                }
                Outcome::Rejected => {
                    // Rejected transfers remain DRAFT (C4)
                    sqlx::query("UPDATE cash_transfers SET updated_at = now() WHERE id = $1::uuid")
                        .bind(object_id)
                        .execute(&mut **tx)
                        .await
                        .map_err(pos_error::AppError::Database)?;
                }
            }
            Ok(())
        })
    }

    fn on_decided_or_unavailable<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        request: &'a ApprovalRequest,
        outcome: Outcome,
    ) -> BoxFuture<'a, Result<(), SubjectError>> {
        Box::pin(async move {
            if outcome == Outcome::Approved {
                let t = PgTransferRepository::new(self._pool.clone())
                    .find_by_id_in(tx, &request.object_id)
                    .await
                    .map_err(SubjectError::Failed)?
                    .ok_or_else(|| {
                        SubjectError::Failed(AppError::NotFound {
                            entity: "CashTransfer",
                            id: request.object_id.clone(),
                        })
                    })?;
                let ends = TransferKinds::standard().get(t.kind).ok_or_else(|| {
                    SubjectError::Failed(AppError::Internal(format!(
                        "unregistered transfer kind {:?}",
                        t.kind
                    )))
                })?;
                let reqs = ends
                    .on_dispatch(&t)
                    .map_err(|e| SubjectError::Failed(AppError::Domain(e.to_string())))?;
                if let Err(e) = LedgerCashPostings::new().check_in(tx, &reqs).await {
                    if let AppError::Domain(ref m) = e
                        && m.starts_with("stockInsufficient")
                    {
                        return Err(SubjectError::Refused {
                            code: "stockInsufficient",
                            message: shortfall_message(&StockShortfall::from_refusal(m)),
                        });
                    }
                    return Err(SubjectError::Failed(e));
                }
            }
            self.on_decided(tx, &request.object_id, outcome)
                .await
                .map_err(SubjectError::Failed)
        })
    }
}

fn shortfall_message(s: &StockShortfall) -> String {
    let items: Vec<String> = s
        .lines
        .iter()
        .map(|l| {
            let place = match l.place {
                ShortPlace::Drawer => "ลิ้นชัก",
                ShortPlace::Safe => "ตู้เซฟ",
                ShortPlace::Position => "สต็อก",
            };
            format!(
                "{} {place} ขาด {}",
                l.currency.as_str(),
                l.short_by.normalize()
            )
        })
        .collect();
    format!("ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: {}", items.join(", "))
}

#[derive(Clone, Default, Debug)]
pub struct TransferApprovalPolicy {
    version: String,
}

impl TransferApprovalPolicy {
    pub fn new() -> Self {
        Self {
            version: "1".to_string(),
        }
    }
}

impl ApprovalPolicy for TransferApprovalPolicy {
    /// A5 with ruling Q2: within the limit only a BRANCH_MANAGER of the source branch decides (FINANCE does
    /// not, though it holds `ApprovalsDecide`); above it, or with no limit, only EXECUTIVE (the v3 owner).
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let within_limit = request
            .proposed_payload
            .get("within_limit")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let from_branch = request
            .proposed_payload
            .get("from_branch")
            .and_then(|v| v.as_str());

        if !within_limit {
            if !ApprovalDecider::holds_on(
                who,
                Permission::ApproveCashTransferAboveLimit,
                from_branch,
            ) {
                return Err(Refusal::new(
                    "ABOVE_LIMIT",
                    "only executive may approve transfers above limit",
                ));
            }
            return Ok(());
        }

        if from_branch.is_none()
            || !ApprovalDecider::holds_on(
                who,
                Permission::ApproveCashTransferWithinLimit,
                from_branch,
            )
        {
            return Err(Refusal::new(
                "WITHIN_LIMIT",
                "within the limit only a branch manager of the source branch approves",
            ));
        }

        Ok(())
    }

    fn may_view(&self, who: &Actor, request: &ApprovalRequest) -> bool {
        let from_branch = request
            .proposed_payload
            .get("from_branch")
            .and_then(|v| v.as_str());
        ApprovalDecider::holds_on(who, Permission::ApprovalsDecide, from_branch)
    }

    fn policy_version(&self) -> String {
        self.version.clone()
    }
}
