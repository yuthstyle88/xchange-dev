//! `SafeMove`: moves between drawer and safe (DRAWER_TO_SAFE, SAFE_TO_DRAWER) (D:§4.5, §9.1 CA-6).

use std::collections::HashMap;

use xc_auth::domain::access::{Actor, Permission};
use xc_kernel::money::ForeignAmount;

use crate::domain::ledger::posting::SourceType;
use crate::domain::posting::{CashPlace, PostingRequest, SourceRef};
use crate::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, TransferError, TransferKind, add_line_minor,
};
use crate::domain::transfer::ends::TransferEnds;
use crate::domain::transfer::kinds::require;

#[derive(Clone, Debug)]
pub struct SafeMove {
    kind: TransferKind,
}

impl SafeMove {
    pub fn new(kind: TransferKind) -> Self {
        match kind {
            TransferKind::DrawerToSafe | TransferKind::SafeToDrawer => Self { kind },
            _ => panic!("SafeMove only supports DrawerToSafe and SafeToDrawer"),
        }
    }
}

impl TransferEnds for SafeMove {
    fn kind(&self) -> TransferKind {
        self.kind
    }

    fn check_create(&self, who: &Actor, t: &NewTransfer) -> Result<(), TransferError> {
        // Ends validation
        if t.to_branch.is_some() {
            return Err(TransferError::Validation(
                "safe moves cannot name a destination branch".into(),
            ));
        }

        match self.kind {
            TransferKind::DrawerToSafe => {
                if t.from_shift.is_none() || t.to_shift.is_some() {
                    return Err(TransferError::Validation(
                        "drawer to safe requires from_shift and no to_shift".into(),
                    ));
                }
            }
            TransferKind::SafeToDrawer => {
                if t.from_shift.is_some() || t.to_shift.is_none() {
                    return Err(TransferError::Validation(
                        "safe to drawer requires to_shift and no from_shift".into(),
                    ));
                }
            }
            _ => unreachable!(),
        }

        // Create: `CashManage` on the branch (D:§3.2).
        require(
            who,
            Permission::CashManage,
            &t.from_branch,
            "creating a safe move",
        )
    }

    fn check_receive(&self, who: &Actor, t: &CashTransfer) -> Result<(), TransferError> {
        match self.kind {
            // Cash enters the safe: `CashManage` on the branch (D:§3.2).
            TransferKind::DrawerToSafe => require(
                who,
                Permission::CashManage,
                &t.from_branch,
                "receiving into the safe",
            ),
            // Cash enters a drawer: `ShiftOperate` on the branch here; the person rule, that `who` is the
            // teller of `to_shift` (ruling Q1), needs the shift row and is `TransferFlow::receive`'s.
            TransferKind::SafeToDrawer => {
                if t.to_shift.is_none() {
                    return Err(TransferError::Validation(
                        "safe to drawer requires to_shift".into(),
                    ));
                }
                require(
                    who,
                    Permission::ShiftOperate,
                    &t.from_branch,
                    "receiving into a drawer",
                )
            }
            _ => unreachable!(),
        }
    }

    fn on_dispatch(&self, t: &CashTransfer) -> Result<Vec<PostingRequest>, TransferError> {
        let from_place = match self.kind {
            TransferKind::DrawerToSafe => {
                CashPlace::Drawer(t.from_shift.clone().expect("from_shift required"))
            }
            TransferKind::SafeToDrawer => CashPlace::Safe(t.from_branch.clone()),
            _ => unreachable!(),
        };

        let mut totals: HashMap<_, i64> = HashMap::new();
        for line in &t.lines {
            if line.qty_sent > 0 {
                add_line_minor(&mut totals, line.currency, line.face_value, line.qty_sent)?;
            }
        }

        Ok(totals
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
            .collect())
    }

    fn on_receive(&self, t: &CashTransfer) -> Result<Vec<PostingRequest>, TransferError> {
        let to_place = match self.kind {
            TransferKind::DrawerToSafe => CashPlace::Safe(t.from_branch.clone()),
            TransferKind::SafeToDrawer => {
                CashPlace::Drawer(t.to_shift.clone().expect("to_shift required"))
            }
            _ => unreachable!(),
        };

        let mut totals: HashMap<_, i64> = HashMap::new();
        for line in &t.lines {
            let qty = line.qty_received.unwrap_or(0);
            if qty > 0 {
                add_line_minor(&mut totals, line.currency, line.face_value, qty)?;
            }
        }

        Ok(totals
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
            .collect())
    }
}
