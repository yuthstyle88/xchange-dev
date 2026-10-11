//! A planner draft (design §4.1, P5): a plan, not a commitment. It holds no quote, stock or rate.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money};

use crate::domain::owner::DraftOwner;

/// The one input of a plan: a foreign amount or a THB budget (`planner_drafts` CHECK).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftAmount {
    /// `quantity`, the foreign amount.
    Quantity(ForeignAmount),
    /// `budget_thb`.
    BudgetThb(Money),
}

/// What the customer plans: the fields of a draft a save may change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftPlan {
    pub branch_id: Option<BranchId>,
    pub currency: CurrencyCode,
    pub customer_side: CustomerSide,
    pub amount: DraftAmount,
}

/// One stored draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerDraft {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub owner: DraftOwner,
    pub plan: DraftPlan,
    pub version: i32,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A save: a create when `id` is `None`, else an update of that draft at `expected_version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSave {
    pub id: Option<Uuid>,
    pub expected_version: Option<i32>,
    pub plan: DraftPlan,
}
