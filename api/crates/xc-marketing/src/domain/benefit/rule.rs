//! `BenefitRule` (design §4.1 `benefit_rules`, §6).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::eligibility::Eligibility;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenefitClass {
    #[serde(rename = "PRICE")]
    Price,
    #[serde(rename = "REWARD")]
    Reward,
}

impl BenefitClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            BenefitClass::Price => "PRICE",
            BenefitClass::Reward => "REWARD",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BenefitCaps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_transaction_max_thb: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_customer_uses: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_customer_period_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenefitRule {
    pub id: Uuid,
    pub campaign_version_id: Uuid,
    pub benefit_type: String,
    pub benefit_class: BenefitClass,
    pub eligibility: Eligibility,
    pub calculation: serde_json::Value,
    pub caps: BenefitCaps,
    pub requires_coupon: bool,
    pub rule_no: i32,
}
