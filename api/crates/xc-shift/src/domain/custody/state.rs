//! Custody state.

use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::CashException;
use crate::domain::custody::entities::customer_hold::CustomerHold;
use crate::domain::custody::entities::tx_draft::TxDraft;
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::entity::Entity;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::shift::value_objects::balances::Balances;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyState {
    pub phase: Phase,
    pub safe: Option<Balances>,
    pub target: Option<CashTarget>,
    pub entity: Option<Entity>,
    pub attendance: Vec<Attendance>,
    pub cutoff: Option<Cutoff>,
    pub exceptions: Vec<CashException>,
    pub holds: Vec<CustomerHold>,
    pub drafts: Vec<TxDraft>,
    pub service_opened_at: Option<CustodyTime>,
}

impl CustodyState {
    pub fn initial() -> CustodyState {
        CustodyState {
            phase: Phase::Closed,
            safe: None,
            target: None,
            entity: None,
            attendance: Vec::new(),
            cutoff: None,
            exceptions: Vec::new(),
            holds: Vec::new(),
            drafts: Vec::new(),
            service_opened_at: None,
        }
    }
}
