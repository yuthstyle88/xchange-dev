//! Shared v3 DTO types (§7.4).

use std::path::Path;

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// An answer from a view polling endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct ViewAnswer<T: TS> {
    #[ts(type = "number")]
    pub version: i64,
    pub changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub data: Option<T>,
}

/// Refusal envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct V3ErrorBody {
    pub error: V3ErrorDetail,
}

/// Details of a refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct V3ErrorDetail {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub field: Option<String>,
    pub message: String,
}

/// Decimal money string (W7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export_to = "common.ts")]
pub struct Money(pub String);

/// Decimal rate string (W7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export_to = "common.ts")]
pub struct Rate(pub String);

/// Current user / actor information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct Me {
    // `users.id` (#1030 §3.3).
    pub id: String,
    // `users.name`; `null` when the caller has no `users` row (#1030 §3.3).
    pub name: Option<String>,
    // The caller's valid-now role rows, ordered by role name, then branch id (#1030 §3.3).
    pub roles: Vec<MeRole>,
    // The `Permission` names `Authorizer::check` grants at `branch`, sorted (#1030 §3.3).
    pub permissions: Vec<String>,
    // The branch this request works in: the session branch (SB-1 §14.3).
    pub branch: String,
    // The name of `branch` (SB-1 §14.4).
    pub branch_name: String,
    // S: the branches this user may switch to, ordered by code, then id (SB-1 §14.2).
    pub branches: Vec<BranchRef>,
}

/// One role row of `Me.roles`: the role's wire name and the branch it holds at, `null` organisation-wide
/// (#1030 §3.3, the sign-in session route's shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct MeRole {
    pub role: String,
    pub branch_id: Option<String>,
}

/// One branch the session may switch to (SB-1 §14.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export_to = "common.ts")]
pub struct BranchRef {
    pub id: String,
    pub code: String,
    pub name: String,
}

/// Empty filter for views without one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export_to = "common.ts")]
pub struct NoFilter {}

impl ViewFilter for NoFilter {}

/// Period filter with inclusive YYYY-MM-DD bounds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct Period {
    pub from: String,
    pub to: String,
}

impl Period {
    pub fn check(&self) -> Result<(), V3Error> {
        if !self.from.is_empty() && !is_valid_yyyy_mm_dd(&self.from) {
            return Err(V3Error::new(
                "invalidRequest",
                Some("from"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        if !self.to.is_empty() && !is_valid_yyyy_mm_dd(&self.to) {
            return Err(V3Error::new(
                "invalidRequest",
                Some("to"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        if !self.from.is_empty() && !self.to.is_empty() && self.from > self.to {
            return Err(V3Error::new(
                "periodOrder",
                Some("to"),
                "วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

fn is_valid_yyyy_mm_dd(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    for (i, &b) in bytes.iter().enumerate() {
        if i != 4 && i != 7 && !b.is_ascii_digit() {
            return false;
        }
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// Rate freshness facts (§4.10, RF-1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct RateFreshnessDto {
    pub stale: bool,
    pub source_date: Option<String>,
    pub published_at: Option<String>,
}

/// Exports shared v3 types to `common.ts` in `dir`.
pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ViewAnswer::<()>::export_all(&cfg)?;
    V3ErrorBody::export_all(&cfg)?;
    Money::export_all(&cfg)?;
    Rate::export_all(&cfg)?;
    Me::export_all(&cfg)?;
    BranchRef::export_all(&cfg)?;
    NoFilter::export_all(&cfg)?;
    Period::export_all(&cfg)?;
    RateFreshnessDto::export_all(&cfg)?;
    Ok(())
}
