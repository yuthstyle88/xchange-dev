//! `CampaignVersionSubject` and `CampaignApprovalPolicy` (MK-4, design `marketing-campaigns.md` §4.4, §6, D-5).

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role};

pub struct CampaignVersionSubject {
    _pool: PgPool,
}

impl CampaignVersionSubject {
    pub fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }
}

impl ApprovalSubject for CampaignVersionSubject {
    fn type_name(&self) -> &'static str {
        "CAMPAIGN_VERSION"
    }

    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let version_id = Uuid::parse_str(object_id)
                .map_err(|e| pos_error::AppError::Validation(format!("invalid uuid: {e}")))?;
            let hash: Option<String> =
                sqlx::query_scalar("SELECT version_hash FROM campaign_versions WHERE id = $1")
                    .bind(version_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| pos_error::AppError::Internal(e.to_string()))?
                    .flatten();

            Ok(hash.unwrap_or_default())
        })
    }

    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let version_id = Uuid::parse_str(object_id)
                .map_err(|e| pos_error::AppError::Validation(format!("invalid uuid: {e}")))?;

            match outcome {
                Outcome::Approved => {
                    let row = sqlx::query(
                        "SELECT organization_id, campaign_id FROM campaign_versions WHERE id = $1",
                    )
                    .bind(version_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

                    let Some(r) = row else {
                        return Err(pos_error::AppError::NotFound {
                            entity: "campaign_version",
                            id: version_id.to_string(),
                        });
                    };
                    let org_id: Uuid = r.get("organization_id");
                    let campaign_id: Uuid = r.get("campaign_id");

                    // Supersede previous approved or paused versions (D-3, D-5)
                    sqlx::query(
                        "UPDATE campaign_versions SET status = 'SUPERSEDED' \
                         WHERE organization_id = $1 AND campaign_id = $2 AND status IN ('APPROVED', 'PAUSED') AND id <> $3",
                    )
                    .bind(org_id)
                    .bind(campaign_id)
                    .bind(version_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

                    // Mark this version APPROVED
                    sqlx::query(
                        "UPDATE campaign_versions SET status = 'APPROVED' WHERE organization_id = $1 AND id = $2",
                    )
                    .bind(org_id)
                    .bind(version_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

                    // Move campaigns.current_version_id
                    sqlx::query(
                        "UPDATE campaigns SET current_version_id = $1 WHERE organization_id = $2 AND id = $3",
                    )
                    .bind(version_id)
                    .bind(org_id)
                    .bind(campaign_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;
                }
                Outcome::Rejected => {
                    sqlx::query("UPDATE campaign_versions SET status = 'REJECTED' WHERE id = $1")
                        .bind(version_id)
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;
                }
            }

            Ok(())
        })
    }
}

pub struct CampaignApprovalPolicy {
    _pool: PgPool,
}

impl CampaignApprovalPolicy {
    pub fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }
}

impl ApprovalPolicy for CampaignApprovalPolicy {
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        // D-5: proposer never decides
        if request.requested_by == who.user_id {
            return Err(Refusal::new(
                "proposerCannotDecide",
                "proposer cannot decide their own request",
            ));
        }

        // D-5: only EXECUTIVE decides
        if !who.roles.iter().any(|r| r.role == Role::Executive) {
            return Err(Refusal::new(
                "outOfScope",
                "only executive may decide campaign versions",
            ));
        }

        Ok(())
    }

    fn allows_self_decision(&self, _who: &Actor) -> bool {
        // D-5: proposer never decides
        false
    }

    fn policy_version(&self) -> String {
        "1".to_string()
    }
}

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    let subject = Arc::new(CampaignVersionSubject::new(pool.clone()));
    let policy = Arc::new(CampaignApprovalPolicy::new(pool));
    registry.register(subject, policy);
}
