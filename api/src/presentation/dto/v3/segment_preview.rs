//! MK-6 (#1404): `segment_preview` view DTO and filter (ruling Q2, issuecomment-6121204241).

use std::path::Path;

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "segment_preview.ts")]
pub struct SegmentPreviewFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
}

impl ViewFilter for SegmentPreviewFilter {
    fn check(&self) -> Result<(), V3Error> {
        if let Some(ref id) = self.id
            && Uuid::parse_str(id).is_err()
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("id"),
                "id ต้องเป็น uuid",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "segment_preview.ts")]
pub struct MemberMetricsDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recency_days: Option<u32>,
    pub frequency: u32,
    pub value_thb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tier_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "segment_preview.ts")]
pub struct MaskedMemberSample {
    pub customer_id: String,
    pub masked_name: String,
    pub metrics: MemberMetricsDto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "segment_preview.ts")]
pub struct MetricDistribution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub avg_recency_days: Option<f64>,
    pub avg_frequency: f64,
    pub total_value_thb: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "segment_preview.ts")]
pub struct SegmentPreviewDto {
    pub snapshot_id: String,
    pub member_count: usize,
    pub distribution: MetricDistribution,
    pub sample: Vec<MaskedMemberSample>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    SegmentPreviewFilter::export_all(&cfg)?;
    MemberMetricsDto::export_all(&cfg)?;
    MaskedMemberSample::export_all(&cfg)?;
    MetricDistribution::export_all(&cfg)?;
    SegmentPreviewDto::export_all(&cfg)?;
    Ok(())
}
