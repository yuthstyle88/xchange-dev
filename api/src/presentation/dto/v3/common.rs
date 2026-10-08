//! Shared v3 DTO types (§7.4).

use std::path::Path;

use serde::Serialize;
use ts_rs::TS;

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
    pub kind: String,
    pub id: String,
    pub role: String,
    pub branch: String,
}

/// Exports shared v3 types to `common.ts` in `dir`.
pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ViewAnswer::<()>::export_all(&cfg)?;
    V3ErrorBody::export_all(&cfg)?;
    Money::export_all(&cfg)?;
    Rate::export_all(&cfg)?;
    Me::export_all(&cfg)?;
    Ok(())
}
