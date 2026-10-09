//! Admin slips view DTOs for v3 (§7, §12.1).

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Period;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Transaction status for filtering slips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "admin_slips.ts")]
pub enum AdminSlipStatus {
    COMPLETED,
    CANCELLED,
    PENDING,
}

/// Filter for the `admin_slips` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_slips.ts")]
pub struct AdminSlipsFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub q: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "AdminSlipStatus")]
    pub status: Option<String>,
}

impl ViewFilter for AdminSlipsFilter {
    fn check(&self) -> Result<(), V3Error> {
        let period = Period {
            from: self.from.clone().unwrap_or_default(),
            to: self.to.clone().unwrap_or_default(),
        };
        period.check()?;

        if let Some(ref q) = self.q
            && q.chars().count() > 100
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("q"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

        if let Some(ref s) = self.status
            && !s.is_empty()
            && s != "COMPLETED"
            && s != "CANCELLED"
            && s != "PENDING"
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("status"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

        Ok(())
    }
}
