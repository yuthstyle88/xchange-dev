//! The inventory's persistence port (design §8). Scaffold by the ticket author.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use super::weighted_average::WeightedAverage;
use crate::domain::ledger::posting::PostingId;

/// `inventory_movements.movement_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MovementType {
    Acquire,
    Dispose,
    TransferOut,
    TransferIn,
    Adjust,
}

impl MovementType {
    /// `ACQUIRE`, `DISPOSE`, `TRANSFER_OUT`, `TRANSFER_IN`, `ADJUST`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acquire => "ACQUIRE",
            Self::Dispose => "DISPOSE",
            Self::TransferOut => "TRANSFER_OUT",
            Self::TransferIn => "TRANSFER_IN",
            Self::Adjust => "ADJUST",
        }
    }
}

/// One `inventory_lots` row (an acquisition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lot {
    pub id: Uuid,
    pub organization_id: String,
    pub branch: BranchId,
    pub currency: CurrencyCode,
    pub denomination_id: Option<i64>,
    pub acquired_at: DateTime<Utc>,
    pub quantity: Decimal,
    pub unit_cost_thb: Decimal,
    pub source_posting_id: PostingId,
}

/// One `inventory_movements` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movement {
    pub organization_id: String,
    pub branch: BranchId,
    pub currency: CurrencyCode,
    pub lot_id: Option<Uuid>,
    pub posting_id: PostingId,
    pub transaction_line_id: Option<Uuid>,
    pub movement_type: MovementType,
    pub quantity_signed: Decimal,
    pub cost_thb_signed: Decimal,
    pub average_after: Option<Decimal>,
    pub created_at: DateTime<Utc>,
}

pub trait InventoryRepository: Send + Sync {
    /// The position `(org, branch, currency)`, locked for update inside `tx`. No row yet ⇒
    /// [`WeightedAverage::EMPTY`].
    fn position_for_update_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        branch: &'a BranchId,
        currency: CurrencyCode,
    ) -> BoxFuture<'a, AppResult<WeightedAverage>>;

    /// Inserts `lot` (when given) and `movement`, and moves the position by the movement's
    /// `quantity_signed` and `cost_thb_signed` (creating it, `version` 1, or bumping `version`).
    fn record_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        movement: &'a Movement,
        lot: Option<&'a Lot>,
    ) -> BoxFuture<'a, AppResult<()>>;
}
