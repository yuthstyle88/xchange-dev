//! Custody commands.

use crate::domain::custody::commands::proposal::Proposal;
use crate::domain::shift::value_objects::balances::Balances;
use xc_kernel::money::{CurrencyCode, Money, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockDirection {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustodyCommand {
    SetEntity {
        company: String,
        site: String,
        confirm_separated: bool,
    },
    Enable,
    Clock(ClockDirection),
    BeginTransaction {
        side: Side,
        currency: CurrencyCode,
        amount: Money,
    },
    SetTarget {
        amounts: Balances,
    },
    StartService {
        equipment_ready: bool,
        rates_verified: bool,
        rate_updated: String,
    },
    Cutoff,
    Propose(Box<Proposal>),
    Hold {
        customer_id: String,
        reason: String,
    },
    Release {
        hold_id: String,
        note: String,
    },
    Reject {
        id: String,
        reason: String,
    },
    Sign {
        id: String,
    },
    Dispatch {
        id: String,
    },
    Commit {
        id: String,
    },
    Reconcile {
        id: String,
        note: String,
    },
}
