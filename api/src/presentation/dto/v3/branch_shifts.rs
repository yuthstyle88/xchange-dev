//! Ticket CS-9 (#6188): branch shifts DTOs for `branchShiftsSave` and `branch_shifts`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_shift::domain::shift_plan::shift_plan::ShiftPlan;

/// One shift template row in `branchShiftsSave` input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_shifts.ts")]
pub struct ShiftTemplateSaveRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub id: Option<String>,
    pub name: String,
    pub starts_at: String,
    pub ends_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub break_starts_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub break_ends_at: Option<String>,
}

/// The input payload for `branchShiftsSave`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_shifts.ts")]
pub struct BranchShiftsSaveInput {
    pub branch: String,
    pub templates: Vec<ShiftTemplateSaveRow>,
}

/// One shift template row in `branch_shifts` view and `branchShiftsSave` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_shifts.ts")]
pub struct ShiftTemplateViewRow {
    pub id: String,
    pub name: String,
    pub starts_at: String,
    pub ends_at: String,
    pub break_starts_at: Option<String>,
    pub break_ends_at: Option<String>,
}

/// The answer for `branch_shifts` view and `branchShiftsSave`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_shifts.ts")]
pub struct BranchShiftsData {
    pub templates: Vec<ShiftTemplateViewRow>,
    pub cutover: String,
    pub default_shift_hours: u32,
}

impl BranchShiftsData {
    pub fn from_plan(plan: &ShiftPlan) -> Self {
        let templates = plan
            .templates()
            .iter()
            .map(|t| ShiftTemplateViewRow {
                id: t.id.clone(),
                name: t.name.clone(),
                starts_at: t.starts_at.format("%H:%M").to_string(),
                ends_at: t.ends_at.format("%H:%M").to_string(),
                break_starts_at: t.break_window.map(|(bs, _)| bs.format("%H:%M").to_string()),
                break_ends_at: t.break_window.map(|(_, be)| be.format("%H:%M").to_string()),
            })
            .collect();

        let cutover = plan
            .templates()
            .iter()
            .map(|t| t.starts_at)
            .min()
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_else(|| "06:00".to_string());

        Self {
            templates,
            cutover,
            default_shift_hours: ShiftPlan::DEFAULT_SHIFT_HOURS,
        }
    }
}

pub fn export_types(dir: &std::path::Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ShiftTemplateSaveRow::export_all(&cfg)?;
    ShiftTemplateViewRow::export_all(&cfg)?;
    BranchShiftsSaveInput::export_all(&cfg)?;
    BranchShiftsData::export_all(&cfg)?;
    Ok(())
}
