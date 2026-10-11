//! `ExternalMove`: external in/out moves (EXTERNAL_IN, EXTERNAL_OUT) (D:§4.5, §9.1 CA-6).

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

#[derive(Clone, Debug)]
pub struct ExternalMove {
    kind: TransferKind,
}

impl ExternalMove {
    pub fn new(kind: TransferKind) -> Self {
        match kind {
            TransferKind::ExternalIn | TransferKind::ExternalOut => Self { kind },
            _ => panic!("ExternalMove only supports ExternalIn and ExternalOut"),
        }
    }
}

impl TransferEnds for ExternalMove {
    fn kind(&self) -> TransferKind {
        self.kind
    }

    fn check_create(&self, who: &Actor, t: &NewTransfer) -> Result<(), TransferError> {
        // Ends validation: external moves have no to_branch, no shifts
        if t.to_branch.is_some() || t.from_shift.is_some() || t.to_shift.is_some() {
            return Err(TransferError::Validation(
                "external moves must not specify to_branch, from_shift, or to_shift".into(),
            ));
        }

        // Create: `CashManage` on the branch (D:§3.2).
        require(
            who,
            Permission::CashManage,
            &t.from_branch,
            "creating an external move",
        )
    }

    fn check_receive(&self, who: &Actor, t: &CashTransfer) -> Result<(), TransferError> {
        // Receive: `CashManage` on the branch (D:§3.2).
        require(
            who,
            Permission::CashManage,
            &t.from_branch,
            "receiving an external move",
        )
    }

    fn on_dispatch(&self, t: &CashTransfer) -> Vec<PostingRequest> {
        let from_place = match self.kind {
            TransferKind::ExternalIn => CashPlace::Bank,
            TransferKind::ExternalOut => CashPlace::Safe(t.from_branch.clone()),
            _ => unreachable!(),
        };

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
                from: from_place.clone(),
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
        let to_place = match self.kind {
            TransferKind::ExternalIn => CashPlace::Safe(t.from_branch.clone()),
            TransferKind::ExternalOut => CashPlace::Bank,
            _ => unreachable!(),
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
                to: to_place.clone(),
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
