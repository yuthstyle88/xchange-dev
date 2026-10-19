//! C05 (#3433): the `my_benefits` view DTOs (design `marketing-campaigns.md` §6.3, O-3):
//! a customer's own usable personal coupons (never the code or hash) and open APP benefits.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_benefits.ts")]
pub struct BranchRefDto {
    pub branch_code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_benefits.ts")]
pub struct BenefitSummaryDto {
    pub benefit_type: String,
    #[ts(type = "\"PRICE\" | \"REWARD\"")]
    pub benefit_class: String,
    pub delta: Option<String>,
    pub percent: Option<String>,
    pub amount: Option<String>,
    pub fixed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_benefits.ts")]
pub struct BenefitConditionsDto {
    pub min_thb: Option<String>,
    pub currencies: Option<Vec<String>>,
    #[ts(type = "\"BUY\" | \"SELL\" | null")]
    pub shop_side: Option<String>,
    pub branches: Vec<BranchRefDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_benefits.ts")]
pub struct MyCouponDto {
    #[ts(type = "string")]
    pub coupon_id: Uuid,
    pub label: String,
    pub summary: BenefitSummaryDto,
    pub conditions: BenefitConditionsDto,
    #[ts(type = "string")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_benefits.ts")]
pub struct MyBenefitDto {
    #[ts(type = "string")]
    pub rule_id: Uuid,
    pub label: String,
    pub summary: BenefitSummaryDto,
    pub conditions: BenefitConditionsDto,
    #[ts(type = "string")]
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_benefits.ts")]
pub struct MyBenefitsData {
    pub coupons: Vec<MyCouponDto>,
    pub benefits: Vec<MyBenefitDto>,
}
