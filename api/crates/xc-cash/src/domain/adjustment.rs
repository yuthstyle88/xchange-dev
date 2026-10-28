//! A FINANCE ledger adjustment (design `docs/design/blueprint/ledger-inventory.md` §13): what an
//! `ADJUSTMENT` posting corrects, as the `LEDGER_ADJUSTMENT` approval request's `proposed_payload` holds
//! it (AD-6, §13.4). The payload is never updated after insert, so this type is the adjustment.
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): the types and signatures are final; the bodies are
//! LA-2's.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MinorUnit, MoneyDelta};

use crate::domain::ledger::posting::{SourceRef, SourceType};
use crate::domain::posting::{CashPlace, PostingRequest, RevalueRequest, differences};

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
        let branch = payload
            .get("branch")
            .and_then(Value::as_str)
            .ok_or_else(|| AdjustmentInvalid::new("branch", "branch must be a string"))?;
        let reason = payload
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| AdjustmentInvalid::new("reason", "reason must be a string"))?;
        let kind_str = payload
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| AdjustmentInvalid::new("kind", "kind must be QUANTITY or COST"))?;
        let raw_lines = payload
            .get("lines")
            .and_then(Value::as_array)
            .ok_or_else(|| AdjustmentInvalid::new("lines", "lines must be an array"))?;

        let kind = match kind_str {
            "QUANTITY" => {
                let place = match payload.get("place") {
                    None | Some(Value::Null) => String::new(),
                    Some(Value::String(s)) => s.clone(),
                    Some(_) => {
                        return Err(AdjustmentInvalid::new("place", "place must be a string"));
                    }
                };
                let mut lines = Vec::with_capacity(raw_lines.len());
                for (i, item) in raw_lines.iter().enumerate() {
                    let currency = item
                        .get("currency")
                        .and_then(Value::as_str)
                        .and_then(|c| CurrencyCode::parse(c).ok())
                        .ok_or_else(|| {
                            AdjustmentInvalid::new(
                                format!("lines[{i}].currency"),
                                "unknown currency code",
                            )
                        })?;
                    let quantity = item
                        .get("quantity")
                        .and_then(Value::as_str)
                        .and_then(|q| Decimal::from_str(q).ok())
                        .ok_or_else(|| {
                            AdjustmentInvalid::new(
                                format!("lines[{i}].quantity"),
                                "quantity must be a decimal string",
                            )
                        })?;
                    lines.push(QuantityLine { currency, quantity });
                }
                AdjustmentKind::Quantity { place, lines }
            }
            "COST" => {
                let mut lines = Vec::with_capacity(raw_lines.len());
                for (i, item) in raw_lines.iter().enumerate() {
                    let currency = item
                        .get("currency")
                        .and_then(Value::as_str)
                        .and_then(|c| CurrencyCode::parse(c).ok())
                        .ok_or_else(|| {
                            AdjustmentInvalid::new(
                                format!("lines[{i}].currency"),
                                "unknown currency code",
                            )
                        })?;
                    let cost_delta_thb = item
                        .get("costDeltaThb")
                        .and_then(Value::as_str)
                        .and_then(|d| Decimal::from_str(d).ok())
                        .ok_or_else(|| {
                            AdjustmentInvalid::new(
                                format!("lines[{i}].costDeltaThb"),
                                "costDeltaThb must be a decimal string",
                            )
                        })?;
                    lines.push(CostLine {
                        currency,
                        cost_delta_thb,
                    });
                }
                AdjustmentKind::Cost { lines }
            }
            _ => {
                return Err(AdjustmentInvalid::new(
                    "kind",
                    "kind must be QUANTITY or COST",
                ));
            }
        };

        let evidence = match payload.get("evidence") {
            None | Some(Value::Null) => Evidence::default(),
            Some(Value::Object(obj)) => {
                let files = match obj.get("files") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(arr)) => arr
                        .iter()
                        .map(|v| {
                            v.as_str().map(str::to_owned).ok_or_else(|| {
                                AdjustmentInvalid::new(
                                    "evidence.files",
                                    "evidence.files must be strings",
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    Some(_) => {
                        return Err(AdjustmentInvalid::new(
                            "evidence.files",
                            "evidence.files must be an array",
                        ));
                    }
                };
                let reference = match obj.get("reference") {
                    None | Some(Value::Null) => String::new(),
                    Some(Value::String(s)) => s.clone(),
                    Some(_) => {
                        return Err(AdjustmentInvalid::new(
                            "evidence.reference",
                            "evidence.reference must be a string",
                        ));
                    }
                };
                Evidence { files, reference }
            }
            Some(_) => {
                return Err(AdjustmentInvalid::new(
                    "evidence",
                    "evidence must be an object",
                ));
            }
        };

        let exception_id = match payload.get("exceptionId") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => {
                return Err(AdjustmentInvalid::new(
                    "exceptionId",
                    "exceptionId must be a string",
                ));
            }
        };

        Ok(Self {
            id: id.to_owned(),
            branch: BranchId::new(branch),
            kind,
            reason: reason.to_owned(),
            evidence,
            exception_id,
        })
    }

    /// The §13.4 JSON, without the id: `from_payload(&a.id, &a.to_payload()) == a`. Amounts are decimal
    /// strings; `place` is written only for QUANTITY; `exceptionId` only when set.
    pub fn to_payload(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("branch".to_owned(), json!(self.branch.as_str()));
        match &self.kind {
            AdjustmentKind::Quantity { place, lines } => {
                obj.insert("kind".to_owned(), json!("QUANTITY"));
                obj.insert("place".to_owned(), json!(place));
                obj.insert(
                    "lines".to_owned(),
                    Value::Array(
                        lines
                            .iter()
                            .map(|l| {
                                json!({
                                    "currency": l.currency.as_str(),
                                    "quantity": l.quantity.to_string(),
                                })
                            })
                            .collect(),
                    ),
                );
            }
            AdjustmentKind::Cost { lines } => {
                obj.insert("kind".to_owned(), json!("COST"));
                obj.insert(
                    "lines".to_owned(),
                    Value::Array(
                        lines
                            .iter()
                            .map(|l| {
                                json!({
                                    "currency": l.currency.as_str(),
                                    "costDeltaThb": l.cost_delta_thb.to_string(),
                                })
                            })
                            .collect(),
                    ),
                );
            }
        }
        obj.insert("reason".to_owned(), json!(self.reason));
        obj.insert(
            "evidence".to_owned(),
            json!({
                "files": self.evidence.files,
                "reference": self.evidence.reference,
            }),
        );
        if let Some(ex) = &self.exception_id {
            obj.insert("exceptionId".to_owned(), json!(ex));
        }
        Value::Object(obj)
    }

    /// Every §13.4 rule on the values. `minor_units` holds the minor unit of every currency the lines
    /// name (THB included); a currency missing from it is refused on `lines[<i>].currency`.
    pub fn validate(
        &self,
        minor_units: &HashMap<CurrencyCode, MinorUnit>,
    ) -> Result<(), AdjustmentInvalid> {
        match &self.kind {
            AdjustmentKind::Quantity { place, lines } => {
                if place != PLACE_SAFE {
                    return Err(AdjustmentInvalid::new("place", "place must be SAFE"));
                }
                if !(1..=MAX_LINES).contains(&lines.len()) {
                    return Err(AdjustmentInvalid::new("lines", "1..=20 lines required"));
                }
                let mut seen = HashSet::with_capacity(lines.len());
                for (i, line) in lines.iter().enumerate() {
                    if !seen.insert(line.currency) {
                        return Err(AdjustmentInvalid::new(
                            format!("lines[{i}].currency"),
                            "repeated currency",
                        ));
                    }
                    let Some(&minor) = minor_units.get(&line.currency) else {
                        return Err(AdjustmentInvalid::new(
                            format!("lines[{i}].currency"),
                            "currency has no known minor unit",
                        ));
                    };
                    let max_places = u32::from(minor.get()).min(MAX_QUANTITY_PLACES);
                    if line.quantity.is_zero() || line.quantity.normalize().scale() > max_places {
                        return Err(AdjustmentInvalid::new(
                            format!("lines[{i}].quantity"),
                            "quantity must be non-zero and within minor unit places",
                        ));
                    }
                }
            }
            AdjustmentKind::Cost { lines } => {
                if !(1..=MAX_LINES).contains(&lines.len()) {
                    return Err(AdjustmentInvalid::new("lines", "1..=20 lines required"));
                }
                let mut seen = HashSet::with_capacity(lines.len());
                for (i, line) in lines.iter().enumerate() {
                    if !seen.insert(line.currency) {
                        return Err(AdjustmentInvalid::new(
                            format!("lines[{i}].currency"),
                            "repeated currency",
                        ));
                    }
                    if line.currency.is_thb() || !minor_units.contains_key(&line.currency) {
                        return Err(AdjustmentInvalid::new(
                            format!("lines[{i}].currency"),
                            "invalid currency for cost adjustment",
                        ));
                    }
                    if line.cost_delta_thb.is_zero()
                        || line.cost_delta_thb.normalize().scale() > MAX_COST_PLACES
                    {
                        return Err(AdjustmentInvalid::new(
                            format!("lines[{i}].costDeltaThb"),
                            "costDeltaThb must be non-zero and at most 2 decimal places",
                        ));
                    }
                }
            }
        }

        let reason_len = self.reason.trim().chars().count();
        if !(1..=MAX_REASON).contains(&reason_len) {
            return Err(AdjustmentInvalid::new(
                "reason",
                "reason must be 1..=500 characters after trimming",
            ));
        }
        if self.evidence.files.len() > MAX_FILES {
            return Err(AdjustmentInvalid::new(
                "evidence.files",
                "at most 10 evidence files allowed",
            ));
        }
        if self.evidence.reference.chars().count() > MAX_REFERENCE {
            return Err(AdjustmentInvalid::new(
                "evidence.reference",
                "reference must be at most 200 characters",
            ));
        }
        if self.evidence.files.is_empty() && self.evidence.reference.trim().is_empty() {
            return Err(AdjustmentInvalid::new(
                "evidence",
                "at least one file or a non-blank reference is required",
            ));
        }
        Ok(())
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
        match &self.kind {
            AdjustmentKind::Quantity { lines, .. } => {
                let diffs: Vec<(CurrencyCode, Decimal)> =
                    lines.iter().map(|l| (l.currency, l.quantity)).collect();
                differences(
                    &CashPlace::Safe(self.branch.clone()),
                    &CashPlace::Variance(self.branch.clone()),
                    &diffs,
                    &self.source(),
                )
            }
            AdjustmentKind::Cost { .. } => Vec::new(),
        }
    }

    /// A COST adjustment's revaluations, one per line in line order, each under `source()` (§13.6).
    /// Empty for QUANTITY.
    pub fn revalue_requests(&self) -> Vec<RevalueRequest> {
        match &self.kind {
            AdjustmentKind::Cost { lines } => {
                let source = self.source();
                lines
                    .iter()
                    .map(|l| {
                        let mut delta = l.cost_delta_thb;
                        delta.rescale(MAX_COST_PLACES);
                        RevalueRequest {
                            branch: self.branch.clone(),
                            currency: l.currency,
                            cost_delta_thb: MoneyDelta::from_minor(delta.mantissa()),
                            source: source.clone(),
                        }
                    })
                    .collect()
            }
            AdjustmentKind::Quantity { .. } => Vec::new(),
        }
    }
}
