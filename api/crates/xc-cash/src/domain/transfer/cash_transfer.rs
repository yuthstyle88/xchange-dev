//! Cash transfer state machine, entities, value objects, and errors.

use std::collections::HashMap;

use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferKind {
    Branch,
    DrawerToSafe,
    SafeToDrawer,
    ExternalIn,
    ExternalOut,
}

impl TransferKind {
    pub const ALL: [TransferKind; 5] = [
        TransferKind::Branch,
        TransferKind::DrawerToSafe,
        TransferKind::SafeToDrawer,
        TransferKind::ExternalIn,
        TransferKind::ExternalOut,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TransferKind::Branch => "BRANCH",
            TransferKind::DrawerToSafe => "DRAWER_TO_SAFE",
            TransferKind::SafeToDrawer => "SAFE_TO_DRAWER",
            TransferKind::ExternalIn => "EXTERNAL_IN",
            TransferKind::ExternalOut => "EXTERNAL_OUT",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "BRANCH" => Some(TransferKind::Branch),
            "DRAWER_TO_SAFE" => Some(TransferKind::DrawerToSafe),
            "SAFE_TO_DRAWER" => Some(TransferKind::SafeToDrawer),
            "EXTERNAL_IN" => Some(TransferKind::ExternalIn),
            "EXTERNAL_OUT" => Some(TransferKind::ExternalOut),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferStatus {
    Draft,
    Approved,
    Dispatched,
    Received,
    Reconciled,
}

impl TransferStatus {
    pub const ALL: [TransferStatus; 5] = [
        TransferStatus::Draft,
        TransferStatus::Approved,
        TransferStatus::Dispatched,
        TransferStatus::Received,
        TransferStatus::Reconciled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TransferStatus::Draft => "DRAFT",
            TransferStatus::Approved => "APPROVED",
            TransferStatus::Dispatched => "DISPATCHED",
            TransferStatus::Received => "RECEIVED",
            TransferStatus::Reconciled => "RECONCILED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "DRAFT" => Some(TransferStatus::Draft),
            "APPROVED" => Some(TransferStatus::Approved),
            "DISPATCHED" => Some(TransferStatus::Dispatched),
            "RECEIVED" => Some(TransferStatus::Received),
            "RECONCILED" => Some(TransferStatus::Reconciled),
            _ => None,
        }
    }

    pub fn next(self, step: TransferStep) -> Result<TransferStatus, TransferError> {
        match (self, step) {
            (TransferStatus::Draft, TransferStep::Approve) => Ok(TransferStatus::Approved),
            (TransferStatus::Approved, TransferStep::Dispatch) => Ok(TransferStatus::Dispatched),
            (TransferStatus::Dispatched, TransferStep::Receive) => Ok(TransferStatus::Received),
            (TransferStatus::Received, TransferStep::Reconcile) => Ok(TransferStatus::Reconciled),
            _ => Err(TransferError::IllegalTransition { from: self, step }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferStep {
    Approve,
    Dispatch,
    Receive,
    Reconcile,
}

impl TransferStep {
    pub const ALL: [TransferStep; 4] = [
        TransferStep::Approve,
        TransferStep::Dispatch,
        TransferStep::Receive,
        TransferStep::Reconcile,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("illegal transition from {from:?} via {step:?}")]
    IllegalTransition {
        from: TransferStatus,
        step: TransferStep,
    },
    #[error("stock insufficient")]
    StockInsufficient,
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Internal(String),
}

/// Computes `face_value × qty` in checked `i64` hundredths (D:§4.5).
pub fn checked_line_minor(face_value: Money, qty: i32) -> Result<i64, TransferError> {
    let face_minor = face_value
        .try_minor()
        .map_err(|_| TransferError::Validation("transfer face value overflows i64".into()))?;
    face_minor
        .checked_mul(qty as i64)
        .ok_or_else(|| TransferError::Validation("transfer line total overflows i64".into()))
}

/// Adds `face_value × qty` in checked `i64` hundredths to `totals[currency]` (D:§4.5).
pub fn add_line_minor(
    totals: &mut HashMap<CurrencyCode, i64>,
    currency: CurrencyCode,
    face_value: Money,
    qty: i32,
) -> Result<(), TransferError> {
    let line_minor = checked_line_minor(face_value, qty)?;
    let total = totals.entry(currency).or_insert(0);
    *total = total
        .checked_add(line_minor)
        .ok_or_else(|| TransferError::Validation("transfer currency total overflows i64".into()))?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferLine {
    pub currency: CurrencyCode,
    pub denomination_id: i64,
    pub face_value: Money,
    pub qty_sent: i32,
    pub qty_received: Option<i32>,
}

impl TransferLine {
    pub fn sent_minor(&self) -> Result<i64, TransferError> {
        checked_line_minor(self.face_value, self.qty_sent)
    }

    pub fn received_minor(&self) -> Result<i64, TransferError> {
        checked_line_minor(self.face_value, self.qty_received.unwrap_or(0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CashTransfer {
    pub id: String,
    pub kind: TransferKind,
    pub from_branch: BranchId,
    pub to_branch: Option<BranchId>,
    pub from_shift: Option<String>,
    pub to_shift: Option<String>,
    pub status: TransferStatus,
    pub created_by: String,
    pub lines: Vec<TransferLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTransferLine {
    pub currency: CurrencyCode,
    pub denomination_id: i64,
    pub qty: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTransfer {
    pub kind: TransferKind,
    pub from_branch: BranchId,
    pub to_branch: Option<BranchId>,
    pub from_shift: Option<String>,
    pub to_shift: Option<String>,
    pub lines: Vec<NewTransferLine>,
    pub note: Option<String>,
    pub idempotency_key: String,
}
