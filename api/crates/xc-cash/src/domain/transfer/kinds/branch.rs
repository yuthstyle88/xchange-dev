//! `BranchTransferEnds`: transfer between two branches (D:§4.5, §9.1 CA-5).

use std::collections::HashMap;

use xc_auth::domain::access::{Actor, Permission};
use xc_kernel::money::ForeignAmount;

use crate::domain::ledger::posting::SourceType;
use crate::domain::posting::{CashPlace, PostingRequest, SourceRef};
use crate::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, TransferError, TransferKind,
};
use crate::domain::transfer::ends::TransferEnds;
use crate::domain::transfer::kinds::require;

#[derive(Clone, Default, Debug)]
pub struct BranchTransferEnds;

impl BranchTransferEnds {
    pub fn new() -> Self {
        Self
    }
}

impl TransferEnds for BranchTransferEnds {
    fn kind(&self) -> TransferKind {
        TransferKind::Branch
    }

    fn check_create(&self, who: &Actor, t: &NewTransfer) -> Result<(), TransferError> {
        // Ends check: destination branch must be specified and different from source; no shifts
        let dest = t
            .to_branch
            .as_ref()
            .ok_or_else(|| TransferError::Validation("destination branch required".into()))?;
        if dest == &t.from_branch {
            return Err(TransferError::Validation(
                "destination branch cannot be source branch".into(),
            ));
        }
        if t.from_shift.is_some() || t.to_shift.is_some() {
            return Err(TransferError::Validation(
                "branch transfer cannot name shifts".into(),
            ));
        }

        // Create: `CashManage` on the source branch (D:§3.1).
        require(
            who,
            Permission::CashManage,
            &t.from_branch,
            "creating a branch transfer",
        )
    }

    fn check_receive(&self, who: &Actor, t: &CashTransfer) -> Result<(), TransferError> {
        let dest = t
            .to_branch
            .as_ref()
            .ok_or_else(|| TransferError::Validation("destination branch missing".into()))?;

        // Receiver cannot be the creator
        if who.user_id == t.created_by {
            return Err(TransferError::Validation(
                "creator cannot receive own transfer".into(),
            ));
        }

        // Receive: `CashManage` on the destination branch (D:§3.1).
        require(
            who,
            Permission::CashManage,
            dest,
            "receiving a branch transfer",
        )
    }

    fn on_dispatch(&self, t: &CashTransfer) -> Vec<PostingRequest> {
        let mut totals: HashMap<_, i64> = HashMap::new();
        for line in &t.lines {
            if line.qty_sent > 0 {
                let line_minor = line.face_value.minor() * (line.qty_sent as i64);
                *totals.entry(line.currency).or_insert(0) += line_minor;
            }
        }

        totals
            .into_iter()
            .filter(|(_, minor)| *minor > 0)
            .map(|(currency, minor)| PostingRequest {
                from: CashPlace::Safe(t.from_branch.clone()),
                to: CashPlace::InTransit(t.id.clone()),
                currency,
                quantity: ForeignAmount::from_decimal(rust_decimal::Decimal::new(minor, 2)),
                source: SourceRef {
                    source_type: SourceType::Transfer,
                    source_id: t.id.clone(),
                    source_key: "dispatch".to_string(),
                },
                line: None,
                reverses: None,
            })
            .collect()
    }

    fn on_receive(&self, t: &CashTransfer) -> Vec<PostingRequest> {
        let dest = match &t.to_branch {
            Some(b) => b.clone(),
            None => return vec![],
        };

        let mut totals: HashMap<_, i64> = HashMap::new();
        for line in &t.lines {
            let qty = line.qty_received.unwrap_or(0);
            if qty > 0 {
                let line_minor = line.face_value.minor() * (qty as i64);
                *totals.entry(line.currency).or_insert(0) += line_minor;
            }
        }

        totals
            .into_iter()
            .filter(|(_, minor)| *minor > 0)
            .map(|(currency, minor)| PostingRequest {
                from: CashPlace::InTransit(t.id.clone()),
                to: CashPlace::Safe(dest.clone()),
                currency,
                quantity: ForeignAmount::from_decimal(rust_decimal::Decimal::new(minor, 2)),
                source: SourceRef {
                    source_type: SourceType::Transfer,
                    source_id: t.id.clone(),
                    source_key: "receive".to_string(),
                },
                line: None,
                reverses: None,
            })
            .collect()
    }
}
