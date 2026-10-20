//! `PgCustomerBenefits` (C05 #3433, design `marketing-campaigns.md` §6.3, O-3):
//! reads a customer's personal usable coupons (never `code_hash`) and the organisation's open rules.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};
use uuid::Uuid;

use crate::domain::benefit::eligibility::Eligibility;
use crate::domain::benefit::rule::{BenefitCaps, BenefitClass, BenefitRule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    pub branch_code: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct CustomerCoupon {
    pub coupon_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub label: String,
    pub rule: BenefitRule,
    pub branches: Vec<BranchRef>,
}

#[derive(Debug, Clone)]
pub struct OpenRule {
    pub rule: BenefitRule,
    pub version_id: Uuid,
    pub segment_snapshot_id: Option<Uuid>,
    pub ends_at: DateTime<Utc>,
    pub label: String,
    pub branches: Vec<BranchRef>,
}

pub struct PgCustomerBenefits;

fn decode_rule(row: &PgRow) -> AppResult<BenefitRule> {
    let benefit_class_str: String = row.get("benefit_class");
    let benefit_class = match benefit_class_str.as_str() {
        "PRICE" => BenefitClass::Price,
        "REWARD" => BenefitClass::Reward,
        other => {
            return Err(AppError::Internal(format!(
                "unknown benefit class: {other}"
            )));
        }
    };

    let eligibility_val: serde_json::Value = row.get("eligibility");
    let eligibility: Eligibility = serde_json::from_value(eligibility_val)
        .map_err(|e| AppError::Internal(format!("failed to decode eligibility: {e}")))?;

    let calculation: serde_json::Value = row.get("calculation");

    let caps_val: serde_json::Value = row.get("caps");
    let caps: BenefitCaps = serde_json::from_value(caps_val)
        .map_err(|e| AppError::Internal(format!("failed to decode caps: {e}")))?;

    Ok(BenefitRule {
        id: row.get("rule_id"),
        campaign_version_id: row.get("campaign_version_id"),
        benefit_type: row.get("benefit_type"),
        benefit_class,
        eligibility,
        calculation,
        caps,
        requires_coupon: row.get("requires_coupon"),
        rule_no: row.get("rule_no"),
    })
}

async fn branches_for_version(
    tx: &mut Transaction<'_, Postgres>,
    version_id: Uuid,
) -> AppResult<Vec<BranchRef>> {
    let rows = sqlx::query(
        "SELECT b.code AS branch_code, b.name \
         FROM campaign_branches cb \
         JOIN branches b ON b.id = cb.branch_id \
         WHERE cb.campaign_version_id = $1 \
         ORDER BY b.code ASC",
    )
    .bind(version_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| BranchRef {
            branch_code: r.get("branch_code"),
            name: r.get("name"),
        })
        .collect())
}

impl PgCustomerBenefits {
    pub async fn personal_coupons_in(
        tx: &mut Transaction<'_, Postgres>,
        org: Uuid,
        customer: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<CustomerCoupon>> {
        let rows = sqlx::query(
            "SELECT \
                cp.id AS coupon_id, \
                cp.expires_at, \
                c.name AS label, \
                r.id AS rule_id, \
                r.campaign_version_id, \
                r.benefit_type, \
                r.benefit_class, \
                r.eligibility, \
                r.calculation, \
                r.caps, \
                r.requires_coupon, \
                r.rule_no \
             FROM coupons cp \
             JOIN benefit_rules r ON r.id = cp.rule_id \
             JOIN campaign_versions v ON v.id = r.campaign_version_id \
             JOIN campaigns c ON c.id = v.campaign_id AND c.organization_id = v.organization_id \
             WHERE cp.organization_id = $1 \
               AND cp.customer_id = $2 \
               AND cp.status = 'ISSUED' \
               AND cp.expires_at > $3 \
               AND v.organization_id = $1 \
               AND v.status = 'APPROVED' \
               AND v.starts_at <= $3 \
               AND v.ends_at > $3 \
             ORDER BY cp.expires_at ASC, cp.id ASC",
        )
        .bind(org)
        .bind(customer)
        .bind(now)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let mut coupons = Vec::with_capacity(rows.len());
        for row in rows {
            let rule = decode_rule(&row)?;
            let branches = branches_for_version(tx, rule.campaign_version_id).await?;
            coupons.push(CustomerCoupon {
                coupon_id: row.get("coupon_id"),
                expires_at: row.get("expires_at"),
                label: row.get("label"),
                rule,
                branches,
            });
        }
        Ok(coupons)
    }

    pub async fn open_rules_in(
        tx: &mut Transaction<'_, Postgres>,
        org: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<OpenRule>> {
        let rows = sqlx::query(
            "SELECT \
                r.id AS rule_id, \
                r.campaign_version_id, \
                r.benefit_type, \
                r.benefit_class, \
                r.eligibility, \
                r.calculation, \
                r.caps, \
                r.requires_coupon, \
                r.rule_no, \
                v.id AS version_id, \
                v.segment_snapshot_id, \
                v.ends_at, \
                c.name AS label \
             FROM campaign_versions v \
             JOIN campaigns c ON c.id = v.campaign_id AND c.organization_id = v.organization_id \
             JOIN benefit_rules r ON r.campaign_version_id = v.id \
             WHERE v.organization_id = $1 \
               AND v.status = 'APPROVED' \
               AND v.starts_at <= $2 \
               AND v.ends_at > $2 \
               AND r.requires_coupon = false \
               AND EXISTS ( \
                   SELECT 1 FROM campaign_branches cb WHERE cb.campaign_version_id = v.id \
               ) \
             ORDER BY v.ends_at ASC, c.name ASC, r.rule_no ASC",
        )
        .bind(org)
        .bind(now)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let mut open = Vec::with_capacity(rows.len());
        for row in rows {
            let rule = decode_rule(&row)?;
            let version_id: Uuid = row.get("version_id");
            let branches = branches_for_version(tx, version_id).await?;
            open.push(OpenRule {
                rule,
                version_id,
                segment_snapshot_id: row.get("segment_snapshot_id"),
                ends_at: row.get("ends_at"),
                label: row.get("label"),
                branches,
            });
        }
        Ok(open)
    }
}
