//! The `open_shift_choices` view's wire types (ticket CS-10 #6189).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One shift template covering `now` that a teller may open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "open_shift_choices.ts")]
pub struct OpenShiftChoice {
    pub template_id: String,
    pub name: String,
    pub business_date: String,
    pub starts_at: String,
    pub ends_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_starts_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_ends_at: Option<String>,
}

/// The `open_shift_choices` view payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "open_shift_choices.ts")]
pub struct OpenShiftChoices {
    pub choices: Vec<OpenShiftChoice>,
    pub preselect: Option<String>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    OpenShiftChoice::export(&cfg)?;
    OpenShiftChoices::export(&cfg)?;
    Ok(())
}
