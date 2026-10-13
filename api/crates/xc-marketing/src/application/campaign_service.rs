//! `CampaignService` (MK-4, design `marketing-campaigns.md` §4.1, §4.2, §4.4, §4.5, §6, §7).

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::domain::campaign::campaign::Campaign;
use crate::domain::campaign::version::{CampaignVersion, StoredStatus, VersionTerms};
use crate::domain::campaign::version_hash::VersionHash;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CampaignServiceError {
    #[error("simulationRequired")]
    SimulationRequired,
    #[error("staleVersion")]
    StaleVersion,
    #[error("campaignNotFound")]
    CampaignNotFound,
    #[error("versionNotFound")]
    VersionNotFound,
    #[error("outOfScope")]
    OutOfScope,
    #[error("database: {0}")]
    Database(String),
}

impl From<sqlx::Error> for CampaignServiceError {
    fn from(err: sqlx::Error) -> Self {
        CampaignServiceError::Database(err.to_string())
    }
}

pub struct CampaignService {
    pool: PgPool,
}

impl CampaignService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn create(
        &self,
        organization_id: Uuid,
        name: &str,
        terms: VersionTerms,
        branches: &[&str],
        created_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Campaign, CampaignServiceError> {
        let mut tx = self.pool.begin().await?;

        let campaign_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
             VALUES ($1, $2, $3, NULL, $4, $5)",
        )
        .bind(campaign_id)
        .bind(organization_id)
        .bind(name)
        .bind(created_by)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO campaign_versions ( \
                 id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
                 max_uses, stack_group, priority, segment_snapshot_id, version_hash, approval_id, \
                 simulated_at, edited_at, created_by, created_at \
             ) VALUES ( \
                 $1, $2, $3, 1, 'DRAFT', $4, $5, $6, \
                 $7, $8, $9, $10, NULL, NULL, \
                 NULL, $11, $12, $11 \
             )",
        )
        .bind(version_id)
        .bind(organization_id)
        .bind(campaign_id)
        .bind(terms.starts_at)
        .bind(terms.ends_at)
        .bind(terms.budget_thb)
        .bind(terms.max_uses)
        .bind(&terms.stack_group)
        .bind(terms.priority)
        .bind(terms.segment_snapshot_id)
        .bind(now)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;

        for branch in branches {
            sqlx::query(
                "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(version_id)
            .bind(*branch)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
             VALUES ($1, 0, 0)",
        )
        .bind(version_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(Campaign {
            id: campaign_id,
            organization_id,
            name: name.to_string(),
            current_version_id: None,
            created_by,
            created_at: now,
        })
    }

    pub async fn save_draft(
        &self,
        organization_id: Uuid,
        version_id: Uuid,
        terms: VersionTerms,
        branches: &[&str],
        now: DateTime<Utc>,
    ) -> Result<CampaignVersion, CampaignServiceError> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(
            "SELECT id, organization_id, campaign_id, version, status, created_by, created_at \
             FROM campaign_versions WHERE organization_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(organization_id)
        .bind(version_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(r) = row else {
            return Err(CampaignServiceError::VersionNotFound);
        };

        let status_str: String = r.get("status");
        if status_str != "DRAFT" && status_str != "SIMULATED" {
            return Err(CampaignServiceError::StaleVersion);
        }

        let campaign_id: Uuid = r.get("campaign_id");
        let version_num: i32 = r.get("version");
        let created_by: Uuid = r.get("created_by");
        let created_at: DateTime<Utc> = r.get("created_at");

        // D-4: an edit moves SIMULATED back to DRAFT and resets simulated_at
        sqlx::query(
            "UPDATE campaign_versions SET \
                 status = 'DRAFT', \
                 starts_at = $1, \
                 ends_at = $2, \
                 budget_thb = $3, \
                 max_uses = $4, \
                 stack_group = $5, \
                 priority = $6, \
                 segment_snapshot_id = $7, \
                 edited_at = $8, \
                 version_hash = NULL \
             WHERE id = $9",
        )
        .bind(terms.starts_at)
        .bind(terms.ends_at)
        .bind(terms.budget_thb)
        .bind(terms.max_uses)
        .bind(&terms.stack_group)
        .bind(terms.priority)
        .bind(terms.segment_snapshot_id)
        .bind(now)
        .bind(version_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM campaign_branches WHERE campaign_version_id = $1")
            .bind(version_id)
            .execute(&mut *tx)
            .await?;

        for branch in branches {
            sqlx::query(
                "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(version_id)
            .bind(*branch)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(CampaignVersion {
            id: version_id,
            organization_id,
            campaign_id,
            version: version_num,
            status: StoredStatus::Draft,
            terms,
            version_hash: None,
            approval_id: None,
            simulated_at: None,
            edited_at: now,
            created_by,
            created_at,
        })
    }

    pub async fn new_version(
        &self,
        organization_id: Uuid,
        campaign_id: Uuid,
        created_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<CampaignVersion, CampaignServiceError> {
        let mut tx = self.pool.begin().await?;

        let campaign_exists: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM campaigns WHERE organization_id = $1 AND id = $2")
                .bind(organization_id)
                .bind(campaign_id)
                .fetch_optional(&mut *tx)
                .await?;

        if campaign_exists.is_none() {
            return Err(CampaignServiceError::CampaignNotFound);
        }

        // Fetch latest version
        let latest_row = sqlx::query(
            "SELECT id, version, starts_at, ends_at, budget_thb, max_uses, stack_group, priority, segment_snapshot_id \
             FROM campaign_versions WHERE organization_id = $1 AND campaign_id = $2 \
             ORDER BY version DESC LIMIT 1",
        )
        .bind(organization_id)
        .bind(campaign_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (new_version_num, terms, prev_version_id) = match latest_row {
            Some(row) => {
                let v_num: i32 = row.get("version");
                let prev_id: Uuid = row.get("id");
                let t = VersionTerms {
                    starts_at: row.get("starts_at"),
                    ends_at: row.get("ends_at"),
                    budget_thb: row.get("budget_thb"),
                    max_uses: row.get("max_uses"),
                    stack_group: row.get("stack_group"),
                    priority: row.get("priority"),
                    segment_snapshot_id: row.get("segment_snapshot_id"),
                };
                (v_num + 1, t, Some(prev_id))
            }
            None => (
                1,
                VersionTerms {
                    starts_at: now,
                    ends_at: now + chrono::Duration::days(30),
                    budget_thb: rust_decimal::Decimal::ZERO,
                    max_uses: None,
                    stack_group: "DEFAULT".to_string(),
                    priority: 0,
                    segment_snapshot_id: None,
                },
                None,
            ),
        };

        let new_version_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO campaign_versions ( \
                 id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
                 max_uses, stack_group, priority, segment_snapshot_id, version_hash, approval_id, \
                 simulated_at, edited_at, created_by, created_at \
             ) VALUES ( \
                 $1, $2, $3, $4, 'DRAFT', $5, $6, $7, \
                 $8, $9, $10, $11, NULL, NULL, \
                 NULL, $12, $13, $12 \
             )",
        )
        .bind(new_version_id)
        .bind(organization_id)
        .bind(campaign_id)
        .bind(new_version_num)
        .bind(terms.starts_at)
        .bind(terms.ends_at)
        .bind(terms.budget_thb)
        .bind(terms.max_uses)
        .bind(&terms.stack_group)
        .bind(terms.priority)
        .bind(terms.segment_snapshot_id)
        .bind(now)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;

        // Copy branches and rules from previous version if it existed
        if let Some(prev_id) = prev_version_id {
            sqlx::query(
                "INSERT INTO campaign_branches (campaign_version_id, branch_id) \
                 SELECT $1, branch_id FROM campaign_branches WHERE campaign_version_id = $2",
            )
            .bind(new_version_id)
            .bind(prev_id)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no) \
                 SELECT gen_random_uuid(), $1, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
                 FROM benefit_rules WHERE campaign_version_id = $2",
            )
            .bind(new_version_id)
            .bind(prev_id)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
             VALUES ($1, 0, 0)",
        )
        .bind(new_version_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(CampaignVersion {
            id: new_version_id,
            organization_id,
            campaign_id,
            version: new_version_num,
            status: StoredStatus::Draft,
            terms,
            version_hash: None,
            approval_id: None,
            simulated_at: None,
            edited_at: now,
            created_by,
            created_at: now,
        })
    }

    pub async fn submit(
        &self,
        organization_id: Uuid,
        version_id: Uuid,
        submitted_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<CampaignVersion, CampaignServiceError> {
        let mut tx = self.pool.begin().await?;

        let version_row = sqlx::query(
            "SELECT id, organization_id, campaign_id, version, status, starts_at, ends_at, \
                    budget_thb, max_uses, stack_group, priority, segment_snapshot_id, \
                    edited_at, created_by, created_at \
             FROM campaign_versions WHERE organization_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(organization_id)
        .bind(version_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = version_row else {
            return Err(CampaignServiceError::VersionNotFound);
        };

        let edited_at: DateTime<Utc> = row.get("edited_at");

        // D-4: Check newest simulation in campaign_simulations: must exist and simulated_at >= edited_at
        let last_sim_at: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT simulated_at FROM campaign_simulations \
             WHERE campaign_version_id = $1 \
             ORDER BY simulated_at DESC LIMIT 1",
        )
        .bind(version_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(sim_at) = last_sim_at else {
            return Err(CampaignServiceError::SimulationRequired);
        };

        if sim_at < edited_at {
            return Err(CampaignServiceError::SimulationRequired);
        }

        let campaign_id: Uuid = row.get("campaign_id");
        let version_num: i32 = row.get("version");
        let created_by: Uuid = row.get("created_by");
        let created_at: DateTime<Utc> = row.get("created_at");

        let terms = VersionTerms {
            starts_at: row.get("starts_at"),
            ends_at: row.get("ends_at"),
            budget_thb: row.get("budget_thb"),
            max_uses: row.get("max_uses"),
            stack_group: row.get("stack_group"),
            priority: row.get("priority"),
            segment_snapshot_id: row.get("segment_snapshot_id"),
        };

        let temp_version = CampaignVersion {
            id: version_id,
            organization_id,
            campaign_id,
            version: version_num,
            status: StoredStatus::Draft,
            terms: terms.clone(),
            version_hash: None,
            approval_id: None,
            simulated_at: Some(sim_at),
            edited_at,
            created_by,
            created_at,
        };

        // Fetch rules
        let rule_rows = sqlx::query(
            "SELECT benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
             FROM benefit_rules WHERE campaign_version_id = $1 ORDER BY rule_no ASC",
        )
        .bind(version_id)
        .fetch_all(&mut *tx)
        .await?;

        let mut rule_blobs = Vec::new();
        for r in rule_rows {
            let b_type: String = r.get("benefit_type");
            let b_class: String = r.get("benefit_class");
            let elig: serde_json::Value = r.get("eligibility");
            let calc: serde_json::Value = r.get("calculation");
            let caps: serde_json::Value = r.get("caps");
            let req_c: bool = r.get("requires_coupon");
            let rule_no: i32 = r.get("rule_no");
            let blob = serde_json::json!({
                "ruleNo": rule_no,
                "benefitType": b_type,
                "benefitClass": b_class,
                "eligibility": elig,
                "calculation": calc,
                "caps": caps,
                "requiresCoupon": req_c,
            });
            rule_blobs.push(blob.to_string());
        }

        // Fetch branches
        let branch_rows: Vec<String> = sqlx::query_scalar(
            "SELECT branch_id FROM campaign_branches WHERE campaign_version_id = $1 ORDER BY branch_id ASC",
        )
        .bind(version_id)
        .fetch_all(&mut *tx)
        .await?;

        // Compute version_hash (D-3)
        let hash = VersionHash::of(&temp_version, &rule_blobs, &branch_rows);

        // Create approval request
        let approval_id = Uuid::new_v4();
        let idem_key = format!("campaign_version_submit_{version_id}");
        sqlx::query(
            "INSERT INTO approval_requests ( \
                 id, organization_id, type, branch_id, object_id, object_version_hash, \
                 policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key \
             ) VALUES ($1, $2, 'CAMPAIGN_VERSION', NULL, $3, $4, '1', '{}'::jsonb, $5, $6, 'PENDING', $7)",
        )
        .bind(approval_id)
        .bind(organization_id)
        .bind(version_id.to_string())
        .bind(&hash)
        .bind(submitted_by.to_string())
        .bind(now)
        .bind(&idem_key)
        .execute(&mut *tx)
        .await?;

        // Update campaign_versions to PENDING_APPROVAL
        sqlx::query(
            "UPDATE campaign_versions SET \
                 status = 'PENDING_APPROVAL', \
                 version_hash = $1, \
                 approval_id = $2, \
                 edited_at = $3 \
             WHERE id = $4",
        )
        .bind(&hash)
        .bind(approval_id)
        .bind(now)
        .bind(version_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(CampaignVersion {
            id: version_id,
            organization_id,
            campaign_id,
            version: version_num,
            status: StoredStatus::PendingApproval,
            terms,
            version_hash: Some(hash),
            approval_id: Some(approval_id),
            simulated_at: Some(sim_at),
            edited_at: now,
            created_by,
            created_at,
        })
    }

    pub async fn pause(
        &self,
        organization_id: Uuid,
        campaign_id: Uuid,
        who: Uuid,
        now: DateTime<Utc>,
    ) -> Result<CampaignVersion, CampaignServiceError> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(
            "SELECT id, organization_id, campaign_id, version, status, starts_at, ends_at, \
                    budget_thb, max_uses, stack_group, priority, segment_snapshot_id, \
                    version_hash, approval_id, simulated_at, created_by, created_at \
             FROM campaign_versions \
             WHERE organization_id = $1 AND campaign_id = $2 AND status = 'APPROVED' \
             FOR UPDATE",
        )
        .bind(organization_id)
        .bind(campaign_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(r) = row else {
            return Err(CampaignServiceError::VersionNotFound);
        };

        let version_id: Uuid = r.get("id");
        let version_num: i32 = r.get("version");
        let version_hash: Option<String> = r.get("version_hash");
        let approval_id: Option<Uuid> = r.get("approval_id");
        let simulated_at: Option<DateTime<Utc>> = r.get("simulated_at");
        let created_by: Uuid = r.get("created_by");
        let created_at: DateTime<Utc> = r.get("created_at");

        let terms = VersionTerms {
            starts_at: r.get("starts_at"),
            ends_at: r.get("ends_at"),
            budget_thb: r.get("budget_thb"),
            max_uses: r.get("max_uses"),
            stack_group: r.get("stack_group"),
            priority: r.get("priority"),
            segment_snapshot_id: r.get("segment_snapshot_id"),
        };

        let _ = who;

        sqlx::query("UPDATE campaign_versions SET status = 'PAUSED', edited_at = $1 WHERE id = $2")
            .bind(now)
            .bind(version_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(CampaignVersion {
            id: version_id,
            organization_id,
            campaign_id,
            version: version_num,
            status: StoredStatus::Paused,
            terms,
            version_hash,
            approval_id,
            simulated_at,
            edited_at: now,
            created_by,
            created_at,
        })
    }

    pub async fn resume(
        &self,
        organization_id: Uuid,
        campaign_id: Uuid,
        who: Uuid,
        now: DateTime<Utc>,
    ) -> Result<CampaignVersion, CampaignServiceError> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(
            "SELECT id, organization_id, campaign_id, version, status, starts_at, ends_at, \
                    budget_thb, max_uses, stack_group, priority, segment_snapshot_id, \
                    version_hash, approval_id, simulated_at, created_by, created_at \
             FROM campaign_versions \
             WHERE organization_id = $1 AND campaign_id = $2 AND status = 'PAUSED' \
             FOR UPDATE",
        )
        .bind(organization_id)
        .bind(campaign_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(r) = row else {
            return Err(CampaignServiceError::VersionNotFound);
        };

        let version_id: Uuid = r.get("id");
        let version_num: i32 = r.get("version");
        let version_hash: Option<String> = r.get("version_hash");
        let approval_id: Option<Uuid> = r.get("approval_id");
        let simulated_at: Option<DateTime<Utc>> = r.get("simulated_at");
        let created_by: Uuid = r.get("created_by");
        let created_at: DateTime<Utc> = r.get("created_at");

        let terms = VersionTerms {
            starts_at: r.get("starts_at"),
            ends_at: r.get("ends_at"),
            budget_thb: r.get("budget_thb"),
            max_uses: r.get("max_uses"),
            stack_group: r.get("stack_group"),
            priority: r.get("priority"),
            segment_snapshot_id: r.get("segment_snapshot_id"),
        };

        let _ = who;

        // Resume: PAUSED -> APPROVED with no new approval (D-5)
        sqlx::query(
            "UPDATE campaign_versions SET status = 'APPROVED', edited_at = $1 WHERE id = $2",
        )
        .bind(now)
        .bind(version_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(CampaignVersion {
            id: version_id,
            organization_id,
            campaign_id,
            version: version_num,
            status: StoredStatus::Approved,
            terms,
            version_hash,
            approval_id,
            simulated_at,
            edited_at: now,
            created_by,
            created_at,
        })
    }
}
