//! `PgCampaignRepository` (design §6, MK-1 #1390).

use futures::future::BoxFuture;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::domain::campaign::campaign::Campaign;
use crate::domain::campaign::repository::{CampaignRepository, CampaignRepositoryError};
use crate::domain::campaign::version::{CampaignVersion, StoredStatus, VersionTerms};

#[derive(Debug, Clone, Default)]
pub struct PgCampaignRepository;

impl PgCampaignRepository {
    pub fn new() -> Self {
        PgCampaignRepository
    }
}

impl CampaignRepository for PgCampaignRepository {
    fn insert_campaign<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        campaign: &'a Campaign,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(campaign.id)
            .bind(campaign.organization_id)
            .bind(&campaign.name)
            .bind(campaign.current_version_id)
            .bind(campaign.created_by)
            .bind(campaign.created_at)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn insert_version<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        version: &'a CampaignVersion,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO campaign_versions ( \
                    id, organization_id, campaign_id, version, status, \
                    starts_at, ends_at, budget_thb, max_uses, stack_group, priority, \
                    segment_snapshot_id, version_hash, approval_id, simulated_at, edited_at, \
                    created_by, created_at \
                 ) VALUES ( \
                    $1, $2, $3, $4, $5, \
                    $6, $7, $8, $9, $10, $11, \
                    $12, $13, $14, $15, $16, \
                    $17, $18 \
                 )",
            )
            .bind(version.id)
            .bind(version.organization_id)
            .bind(version.campaign_id)
            .bind(version.version)
            .bind(version.status.as_str())
            .bind(version.terms.starts_at)
            .bind(version.terms.ends_at)
            .bind(version.terms.budget_thb)
            .bind(version.terms.max_uses)
            .bind(&version.terms.stack_group)
            .bind(version.terms.priority)
            .bind(version.terms.segment_snapshot_id)
            .bind(&version.version_hash)
            .bind(version.approval_id)
            .bind(version.simulated_at)
            .bind(version.edited_at)
            .bind(version.created_by)
            .bind(version.created_at)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn save_version<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        version: &'a CampaignVersion,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE campaign_versions SET \
                    status = $1, \
                    starts_at = $2, \
                    ends_at = $3, \
                    budget_thb = $4, \
                    max_uses = $5, \
                    stack_group = $6, \
                    priority = $7, \
                    segment_snapshot_id = $8, \
                    version_hash = $9, \
                    approval_id = $10, \
                    simulated_at = $11, \
                    edited_at = $12 \
                 WHERE id = $13 AND organization_id = $14",
            )
            .bind(version.status.as_str())
            .bind(version.terms.starts_at)
            .bind(version.terms.ends_at)
            .bind(version.terms.budget_thb)
            .bind(version.terms.max_uses)
            .bind(&version.terms.stack_group)
            .bind(version.terms.priority)
            .bind(version.terms.segment_snapshot_id)
            .bind(&version.version_hash)
            .bind(version.approval_id)
            .bind(version.simulated_at)
            .bind(version.edited_at)
            .bind(version.id)
            .bind(version.organization_id)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn find_campaign<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<Campaign>, CampaignRepositoryError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, organization_id, name, current_version_id, created_by, created_at \
                 FROM campaigns WHERE organization_id = $1 AND id = $2",
            )
            .bind(organization_id)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;

            let Some(r) = row else { return Ok(None) };

            Ok(Some(Campaign {
                id: r.get("id"),
                organization_id: r.get("organization_id"),
                name: r.get("name"),
                current_version_id: r.get("current_version_id"),
                created_by: r.get("created_by"),
                created_at: r.get("created_at"),
            }))
        })
    }

    fn find_version<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<CampaignVersion>, CampaignRepositoryError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, organization_id, campaign_id, version, status, \
                        starts_at, ends_at, budget_thb, max_uses, stack_group, priority, \
                        segment_snapshot_id, version_hash, approval_id, simulated_at, edited_at, \
                        created_by, created_at \
                 FROM campaign_versions WHERE organization_id = $1 AND id = $2",
            )
            .bind(organization_id)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;

            let Some(r) = row else { return Ok(None) };

            let status_str: String = r.get("status");
            let status = match status_str.as_str() {
                "DRAFT" => StoredStatus::Draft,
                "SIMULATED" => StoredStatus::Simulated,
                "PENDING_APPROVAL" => StoredStatus::PendingApproval,
                "APPROVED" => StoredStatus::Approved,
                "REJECTED" => StoredStatus::Rejected,
                "PAUSED" => StoredStatus::Paused,
                "SUPERSEDED" => StoredStatus::Superseded,
                other => {
                    return Err(CampaignRepositoryError::Database(sqlx::Error::Decode(
                        format!("unknown stored status: {other}").into(),
                    )));
                }
            };

            let terms = VersionTerms {
                starts_at: r.get("starts_at"),
                ends_at: r.get("ends_at"),
                budget_thb: r.get("budget_thb"),
                max_uses: r.get("max_uses"),
                stack_group: r.get("stack_group"),
                priority: r.get("priority"),
                segment_snapshot_id: r.get("segment_snapshot_id"),
            };

            Ok(Some(CampaignVersion {
                id: r.get("id"),
                organization_id: r.get("organization_id"),
                campaign_id: r.get("campaign_id"),
                version: r.get("version"),
                status,
                terms,
                version_hash: r.get("version_hash"),
                approval_id: r.get("approval_id"),
                simulated_at: r.get("simulated_at"),
                edited_at: r.get("edited_at"),
                created_by: r.get("created_by"),
                created_at: r.get("created_at"),
            }))
        })
    }

    fn approve<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        version_id: Uuid,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>> {
        Box::pin(async move {
            let campaign_id: Uuid = sqlx::query_scalar(
                "SELECT campaign_id FROM campaign_versions WHERE organization_id = $1 AND id = $2",
            )
            .bind(organization_id)
            .bind(version_id)
            .fetch_one(&mut **tx)
            .await?;

            sqlx::query(
                "UPDATE campaign_versions SET status = 'SUPERSEDED' \
                 WHERE organization_id = $1 AND campaign_id = $2 AND status IN ('APPROVED', 'PAUSED') AND id <> $3",
            )
            .bind(organization_id)
            .bind(campaign_id)
            .bind(version_id)
            .execute(&mut **tx)
            .await?;

            sqlx::query(
                "UPDATE campaign_versions SET status = 'APPROVED' \
                 WHERE organization_id = $1 AND id = $2",
            )
            .bind(organization_id)
            .bind(version_id)
            .execute(&mut **tx)
            .await?;

            sqlx::query(
                "UPDATE campaigns SET current_version_id = $1 \
                 WHERE organization_id = $2 AND id = $3",
            )
            .bind(version_id)
            .bind(organization_id)
            .bind(campaign_id)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }
}
