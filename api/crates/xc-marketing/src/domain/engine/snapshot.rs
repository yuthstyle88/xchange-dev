//! `BenefitSnapshot` and its JSON representation (design §4.1, §6, §7).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::benefit::rule::BenefitClass;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedBenefit {
    pub rule_id: Uuid,
    pub version_id: Uuid,
    pub benefit_type: String,
    pub benefit_class: BenefitClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupon_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<Uuid>,
    pub amount_thb: String,
    pub rate_before: String,
    pub rate_after: String,
    pub fee_before: String,
    pub fee_after: String,
    pub floor_capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRule {
    pub rule_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenefitSnapshot {
    pub engine_version: u32,
    pub applied: Vec<AppliedBenefit>,
    pub skipped: Vec<SkippedRule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_code: Option<String>,
    pub segment_snapshot_ids: Vec<Uuid>,
}

impl BenefitSnapshot {
    pub fn empty() -> Self {
        Self {
            engine_version: 1,
            applied: Vec::new(),
            skipped: Vec::new(),
            tier_code: None,
            segment_snapshot_ids: Vec::new(),
        }
    }
}
