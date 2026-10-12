//! DTO types for the customer app's planner routes (APP-4 #994, design `customer-app.md` §4.3, §4.4):
//! `plannerDraftSave`, `plannerDraftDelete`, `appSettingsSave`, the compute `planner_preview` and the
//! view `my_planner_drafts`.
//!
//! One plan shape on the wire, `amount: { quantity } | { budgetThb }`, mirroring `DraftAmount`
//! (#994 test review, issuecomment-6126535752). It is inlined into each route's types so a view or an
//! action writes only its own generated file.

use std::path::Path;
use std::str::FromStr;

use actix_web::http::StatusCode;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_app::domain::planner::planner_draft::{DraftAmount, DraftPlan, PlannerDraft};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money};

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// The one input of a plan: a foreign amount or a THB budget (`DraftAmount`), as decimal strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PlannerAmount {
    Quantity(String),
    BudgetThb(String),
}

impl PlannerAmount {
    fn of(amount: DraftAmount) -> Self {
        match amount {
            DraftAmount::Quantity(q) => PlannerAmount::Quantity(q.to_string()),
            DraftAmount::BudgetThb(m) => PlannerAmount::BudgetThb(m.to_string()),
        }
    }

    fn to_domain(&self) -> Result<DraftAmount, V3Error> {
        let positive = |text: &str, field: &'static str| match Decimal::from_str(text.trim()) {
            Ok(value) if value > Decimal::ZERO => Ok(value),
            _ => Err(invalid(field)),
        };
        match self {
            PlannerAmount::Quantity(text) => Ok(DraftAmount::Quantity(ForeignAmount::from_decimal(
                positive(text, "quantity")?,
            ))),
            PlannerAmount::BudgetThb(text) => {
                positive(text, "budgetThb")?;
                Money::parse(text.trim())
                    .map(DraftAmount::BudgetThb)
                    .map_err(|_| invalid("budgetThb"))
            }
        }
    }
}

fn invalid(field: &'static str) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        "ข้อมูลไม่ถูกต้อง",
        StatusCode::BAD_REQUEST,
    )
}

/// Builds the domain plan from the wire's currency, side and amount, at `branch`.
fn plan_of(
    branch: Option<BranchId>,
    currency: &str,
    customer_side: &str,
    amount: &PlannerAmount,
) -> Result<DraftPlan, V3Error> {
    Ok(DraftPlan {
        branch_id: branch,
        currency: CurrencyCode::parse(currency.trim()).map_err(|_| invalid("currency"))?,
        customer_side: CustomerSide::parse(customer_side).ok_or_else(|| invalid("customerSide"))?,
        amount: amount.to_domain()?,
    })
}

// ------------------------------------------------------------ plannerDraftSave

/// `POST /api/app/planner-drafts` (create) and `PUT /api/app/planner-drafts/{id}` (update).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "planner_draft_save.ts")]
pub struct PlannerDraftSaveInput {
    /// The draft, from the update's path; absent on a create.
    #[serde(default)]
    #[ts(optional)]
    pub id: Option<String>,
    /// The branch the plan is for.
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    pub currency: String,
    pub customer_side: String,
    #[ts(inline)]
    pub amount: PlannerAmount,
    /// The draft's current version, on an update.
    #[serde(default)]
    #[ts(optional)]
    pub expected_version: Option<i32>,
}

impl PlannerDraftSaveInput {
    /// The plan at `branch` (the seam's branch when the body names one, else none).
    pub fn plan(&self, branch: &BranchId) -> Result<DraftPlan, V3Error> {
        plan_of(
            self.branch.as_ref().map(|_| branch.clone()),
            &self.currency,
            &self.customer_side,
            &self.amount,
        )
    }
}

/// One planner draft: the save's answer and a row of `my_planner_drafts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_planner_drafts.ts")]
pub struct PlannerDraftDto {
    pub id: String,
    pub branch: Option<String>,
    pub currency: String,
    pub customer_side: String,
    #[ts(inline)]
    pub amount: PlannerAmount,
    pub version: i32,
    pub expires_at: String,
    pub created_at: String,
    pub updated_at: String,
}

impl PlannerDraftDto {
    pub fn of(draft: &PlannerDraft) -> Self {
        Self {
            id: draft.id.to_string(),
            branch: draft.plan.branch_id.as_ref().map(|b| b.as_str().to_owned()),
            currency: draft.plan.currency.as_str().to_owned(),
            customer_side: draft.plan.customer_side.as_str().to_owned(),
            amount: PlannerAmount::of(draft.plan.amount),
            version: draft.version,
            expires_at: draft.expires_at.to_rfc3339(),
            created_at: draft.created_at.to_rfc3339(),
            updated_at: draft.updated_at.to_rfc3339(),
        }
    }
}

// ------------------------------------------------------------ plannerDraftDelete

/// `POST /api/app/planner-drafts/{id}/delete`.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "planner_draft_delete.ts")]
pub struct PlannerDraftDeleteInput {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "planner_draft_delete.ts")]
pub struct PlannerDraftDeleteOutput {
    pub id: String,
}

// ------------------------------------------------------------ appSettingsSave

/// `PUT /api/app-settings` (C4).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "app_settings_save.ts")]
pub struct AppSettingsSaveInput {
    pub planner_draft_ttl_seconds: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "app_settings_save.ts")]
pub struct AppSettingsSaveOutput {
    pub planner_draft_ttl_seconds: u32,
}

// ------------------------------------------------------------ planner_preview

/// `POST /api/v3/compute/planner_preview`; the branch is the endpoint's key.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "planner_preview.ts")]
pub struct PlannerPreviewRequest {
    pub currency: String,
    pub customer_side: String,
    #[ts(inline)]
    pub amount: PlannerAmount,
}

impl PlannerPreviewRequest {
    pub fn plan(&self, branch: &BranchId) -> Result<DraftPlan, V3Error> {
        plan_of(
            Some(branch.clone()),
            &self.currency,
            &self.customer_side,
            &self.amount,
        )
    }
}

/// An estimate, never a commitment (P5): the rate is fixed at the desk.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "planner_preview.ts")]
pub struct PlannerPreviewAnswer {
    pub thb: String,
    pub rate: String,
    pub unit_size: String,
    pub stale: bool,
    pub source_date: Option<String>,
}

// ------------------------------------------------------------ my_planner_drafts

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_planner_drafts.ts")]
pub struct MyPlannerDraftsData {
    pub drafts: Vec<PlannerDraftDto>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "my_planner_drafts.ts")]
pub struct MyPlannerDraftsFilter {}

impl ViewFilter for MyPlannerDraftsFilter {}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PlannerDraftSaveInput::export_all(&cfg)?;
    PlannerDraftDto::export_all(&cfg)?;
    PlannerDraftDeleteInput::export_all(&cfg)?;
    PlannerDraftDeleteOutput::export_all(&cfg)?;
    AppSettingsSaveInput::export_all(&cfg)?;
    AppSettingsSaveOutput::export_all(&cfg)?;
    PlannerPreviewRequest::export_all(&cfg)?;
    PlannerPreviewAnswer::export_all(&cfg)?;
    MyPlannerDraftsData::export_all(&cfg)?;
    MyPlannerDraftsFilter::export_all(&cfg)?;
    Ok(())
}
