//! A FINANCE ledger adjustment (design `docs/design/blueprint/ledger-inventory.md` §13): what an
//! `ADJUSTMENT` posting corrects, as the `LEDGER_ADJUSTMENT` approval request's `proposed_payload` holds
//! it (AD-6, §13.4). The payload is never updated after insert, so this type is the adjustment.
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): the types and signatures are final; the bodies are
//! LA-2's.

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde_json::Value;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MinorUnit};

use crate::domain::ledger::posting::{SourceRef, SourceType};
use crate::domain::posting::{PostingRequest, RevalueRequest};

/// `approval_requests.type` of an adjustment.
pub const LEDGER_ADJUSTMENT: &str = "LEDGER_ADJUSTMENT";

/// Every adjustment id starts with this: `ADJ-<uuid v4>` (AD-6).
pub const ID_PREFIX: &str = "ADJ-";

/// `ledger_postings.source_key` of an adjustment's posting (AD-6).
pub const SOURCE_KEY: &str = "post";

/// At most this many lines, one per currency (§13.4).
pub const MAX_LINES: usize = 20;
/// `reason`: 1..=500 characters after trimming (§13.4).
pub const MAX_REASON: usize = 500;
/// `evidence.files`: 0..=10 keys (§13.4).
pub const MAX_FILES: usize = 10;
/// `evidence.reference`: 0..=200 characters (§13.4).
pub const MAX_REFERENCE: usize = 200;
/// `ledger_entries.quantity_signed numeric(20,4)`: a QUANTITY line has at most 4 places (§13.4).
pub const MAX_QUANTITY_PLACES: u32 = 4;
/// A COST line's `costDeltaThb` has at most 2 places (§13.4).
pub const MAX_COST_PLACES: u32 = 2;

/// The only place a QUANTITY adjustment moves (AD-3, owner OA-1).
pub const PLACE_SAFE: &str = "SAFE";

/// Why a payload is refused: `validationFailed` naming `field`.
///
/// `field` is the payload's JSON path: `kind`, `place`, `lines`, `lines[<i>].currency`,
/// `lines[<i>].quantity`, `lines[<i>].costDeltaThb`, `reason`, `evidence`, `evidence.files`,
/// `evidence.reference`, `exceptionId`, `branch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustmentInvalid {
    pub field: String,
    pub message: String,
}

impl AdjustmentInvalid {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// One QUANTITY line: `quantity` is signed and never 0 (a shortfall is negative).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantityLine {
    pub currency: CurrencyCode,
    pub quantity: Decimal,
}

/// One COST line: `cost_delta_thb` is signed and never 0; never THB (§13.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostLine {
    pub currency: CurrencyCode,
    pub cost_delta_thb: Decimal,
}

/// The two kinds (AD-1). `place` is kept as written so `validate` can refuse anything but `SAFE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdjustmentKind {
    Quantity {
        place: String,
        lines: Vec<QuantityLine>,
    },
    Cost {
        lines: Vec<CostLine>,
    },
}

/// What the number rests on (§13.4, owner OA-6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    /// `files.key`s; the service checks each is a file of the adjustment's branch.
    pub files: Vec<String>,
    pub reference: String,
}

/// One adjustment: its id (`approval_requests.object_id`) and its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerAdjustment {
    pub id: String,
    pub branch: BranchId,
    pub kind: AdjustmentKind,
    pub reason: String,
    pub evidence: Evidence,
    pub exception_id: Option<String>,
}

impl LedgerAdjustment {
    /// Reads `payload` (the §13.4 JSON) as the adjustment `id`. Refuses only what cannot be read: a
    /// missing or non-string `branch` / `reason`, a `kind` other than `QUANTITY` / `COST`, a `lines`
    /// that is not an array, an unknown currency code, or an amount that is not a decimal string. A JSON
    /// `null` reads as the key being absent. Every rule on the values themselves is `validate`'s.
    pub fn from_payload(id: &str, payload: &Value) -> Result<Self, AdjustmentInvalid> {
        let _ = (id, payload);
        todo!("LA-2")
    }

    /// The §13.4 JSON, without the id: `from_payload(&a.id, &a.to_payload()) == a`. Amounts are decimal
    /// strings; `place` is written only for QUANTITY; `exceptionId` only when set.
    pub fn to_payload(&self) -> Value {
        todo!("LA-2")
    }

    /// Every §13.4 rule on the values. `minor_units` holds the minor unit of every currency the lines
    /// name (THB included); a currency missing from it is refused on `lines[<i>].currency`.
    pub fn validate(
        &self,
        minor_units: &HashMap<CurrencyCode, MinorUnit>,
    ) -> Result<(), AdjustmentInvalid> {
        let _ = minor_units;
        todo!("LA-2")
    }

    /// `(ADJUSTMENT, id, "post")`: the source of the adjustment's one posting (AD-6).
    pub fn source(&self) -> SourceRef {
        SourceRef {
            source_type: SourceType::Adjustment,
            source_id: self.id.clone(),
            source_key: SOURCE_KEY.to_owned(),
        }
    }

    /// A QUANTITY adjustment's legs: `differences(SAFE(branch), VARIANCE(branch), lines, source())`
    /// (AD-2). Empty for COST.
    pub fn quantity_requests(&self) -> Vec<PostingRequest> {
        todo!("LA-2")
    }

    /// A COST adjustment's revaluations, one per line in line order, each under `source()` (§13.6).
    /// Empty for QUANTITY.
    pub fn revalue_requests(&self) -> Vec<RevalueRequest> {
        todo!("LA-2")
    }
}
