//! LY-5 (#1408): the `customer_tier` view (design `membership-points.md` §4.6, M03):
//! one customer's mirrored tier.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "customer_tier.ts")]
pub struct CustomerTierData {
    pub tier: Option<String>,
}
