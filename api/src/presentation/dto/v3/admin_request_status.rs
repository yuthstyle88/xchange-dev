//! Admin request status view DTOs for v3 (§7, §12.2).

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_rate::domain::validation::validate_request_id;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Filter for the `admin_request_status` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_request_status.ts")]
pub struct AdminRequestStatusFilter {
    pub request_id: String,
}

impl ViewFilter for AdminRequestStatusFilter {
    fn check(&self) -> Result<(), V3Error> {
        if validate_request_id(&self.request_id).is_err() {
            return Err(V3Error::new(
                "invalidRequest",
                Some("requestId"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

/// Answer data for the `admin_request_status` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_request_status.ts")]
pub struct AdminRequestStatusData {
    pub applied: bool,
}
