//! Shift effects requested by the custody machine.

use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::Direction;
use xc_kernel::money::{CurrencyCode, Money};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShiftEffect {
    OpenShift {
        request_id: Option<String>,
        opening: Balances,
    },
    CloseShift {
        request_id: String,
        shift_id: String,
        expected: Balances,
        counted: Balances,
        note: String,
    },
    MoveStock {
        request_id: Option<String>,
        shift_id: String,
        currency: CurrencyCode,
        direction: Direction,
        amount: Money,
        reason: String,
    },
}
