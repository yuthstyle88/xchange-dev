//! MoveStock command.

use serde_json::Value;
use xc_kernel::money::{CurrencyCode, Money};

use crate::domain::shift::commands::payload::{read_amount, read_request_id, read_text};
use crate::domain::shift::value_objects::shift_error::ShiftError;
use crate::domain::shift::value_objects::stock_delta::Direction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveStock {
    pub request_id: String,
    pub shift_id: String,
    pub currency: CurrencyCode,
    pub direction: Direction,
    pub amount: Money,
    pub reason: String,
}

impl MoveStock {
    pub fn from_payload(p: &Value) -> Result<Self, ShiftError> {
        let request_id = read_request_id(p, "requestId")?;
        let currency_text = read_text(p, "currency", 3, true)?;
        let direction_text = read_text(p, "direction", 3, true)?;
        let amount = read_amount(p, "amount", true)?;
        let reason = read_text(p, "reason", 200, true)?;
        let shift_id = read_text(p, "shiftId", 60, true)?;

        let currency =
            CurrencyCode::parse(&currency_text).map_err(|_| ShiftError::MovementInvalid)?;
        let direction = match direction_text.as_str() {
            "IN" => Direction::In,
            "OUT" => Direction::Out,
            _ => return Err(ShiftError::MovementInvalid),
        };

        Ok(Self {
            request_id,
            shift_id,
            currency,
            direction,
            amount,
            reason,
        })
    }
}
