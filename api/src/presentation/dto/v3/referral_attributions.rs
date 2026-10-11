//! RA-3 (#981, design `broker-seam.md` §4.3): the referral commands' bodies and the
//! `referral_attributions` view's rows.

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// `POST /api/transactions/{id}/referral`. `id` is the exchange's uuid (from the path).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "attach_referral.ts")]
pub struct AttachReferralInput {
    pub id: String,
    pub referral_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "attach_referral.ts")]
pub struct AttachReferralOutput {
    pub capture_id: String,
    /// `ACTIVE`.
    pub status: String,
}

/// `POST /api/referral-attributions/{id}/supersede`. `id` is the capture to replace.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "supersede_referral.ts")]
pub struct SupersedeReferralInput {
    pub id: String,
    pub referral_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "supersede_referral.ts")]
pub struct SupersedeReferralOutput {
    /// The new ACTIVE capture.
    pub capture_id: String,
}

/// `?transaction=<uuid>` narrows the rows to one exchange.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "referral_attributions.ts")]
pub struct ReferralAttributionsFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transaction: Option<String>,
}

/// One `referral_attributions` row as the view shows it. `customerSub` is masked as
/// `Redactor` masks (`***` + the last four characters); no phone is ever shown.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "referral_attributions.ts")]
pub struct ReferralAttributionRow {
    pub id: String,
    pub code: String,
    pub status: String,
    pub source: String,
    pub transaction_id: Option<String>,
    pub booking_id: Option<String>,
    pub customer_sub: Option<String>,
    pub captured_at: String,
    pub captured_by: String,
    pub supersedes_id: Option<String>,
    pub reason: Option<String>,
    pub broker_outcome: Option<String>,
    pub pushed_at: Option<String>,
}

/// A `transaction` that is not a uuid is `400 invalidRequest`, field `transaction`.
impl ViewFilter for ReferralAttributionsFilter {
    fn check(&self) -> Result<(), V3Error> {
        if let Some(t) = &self.transaction
            && uuid::Uuid::parse_str(t).is_err()
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("transaction"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}
