//! `TransferApprovalSubject` and `TransferApprovalPolicy` implementing `xc_approval` traits (D:§4.5, A5).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::v3_roles::blueprint_roles;
use xc_auth::domain::access::{Actor, Permission, RolePermissions};
use xc_auth::domain::policy::value_objects::actor::Actor as PolicyActor;
use xc_auth::domain::policy::value_objects::role::Role as PolicyRole;
use xc_kernel::ids::BranchId;

/// The approval core's actor for `who` acting on a transfer whose source is `source`, or `None` when no role of
/// `who` grants `permission` there. There is no fallback: an actor that maps to nothing is refused by the caller.
///
/// The mapping is the inverse of xc-auth's one v3 → Blueprint table (`blueprint_roles`), restricted to the
/// roles valid at `now` that hold `permission` (`RolePermissions`), so it names no Blueprint role:
/// - an organisation-wide role of the v3 owner → `PolicyActor::owner`;
/// - otherwise the first v3 staff role (`PolicyRole::STAFF` order) with such a role on `source`, or
///   organisation-wide, → `PolicyActor::staff` on `source`. The branch is the transfer's source, which that
///   role's scope covers; none is invented.
pub fn policy_actor(
    who: &Actor,
    source: &BranchId,
    permission: Permission,
    now: DateTime<Utc>,
) -> Option<PolicyActor> {
    let holds = |v3: PolicyRole, org_wide_only: bool| {
        who.roles.iter().any(|r| {
            r.valid_from <= now
                && r.valid_to.is_none_or(|to| now < to)
                && RolePermissions::of(r.role).contains(&permission)
                && blueprint_roles(v3).contains(&r.role)
                && (r.branch.is_none() || (!org_wide_only && r.branch.as_ref() == Some(source)))
        })
    };
    if holds(PolicyRole::Owner, true) {
        return Some(PolicyActor::owner(&who.user_id));
    }
    PolicyRole::STAFF
        .into_iter()
        .find(|&v3| holds(v3, false))
        .and_then(|v3| PolicyActor::staff(&who.user_id, v3, source.clone()))
}

/// Whether `role` is one the v3 → Blueprint table makes a BRANCH_MANAGER (today BACKEND and MANAGER).
fn is_branch_manager(role: PolicyRole) -> bool {
    let manager = blueprint_roles(PolicyRole::Manager);
    blueprint_roles(role).iter().any(|r| manager.contains(r))
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
        "CASH_TRANSFER"
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
    fn may_decide(&self, who: &PolicyActor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let within_limit = request
            .proposed_payload
            .get("within_limit")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !within_limit {
            if who.role() != PolicyRole::Owner {
                return Err(Refusal::new(
                    "ABOVE_LIMIT",
                    "only executive may approve transfers above limit",
                ));
            }
            return Ok(());
        }

        let source = request
            .proposed_payload
            .get("from_branch")
            .and_then(|v| v.as_str());
        let on_source = match (who.branch_id(), source) {
            (Some(branch), Some(source)) => branch.as_str() == source,
            _ => false,
        };
        if !(is_branch_manager(who.role()) && on_source) {
            return Err(Refusal::new(
                "WITHIN_LIMIT",
                "within the limit only a branch manager of the source branch approves",
            ));
        }

        Ok(())
    }

    fn policy_version(&self) -> String {
        self.version.clone()
    }
}
