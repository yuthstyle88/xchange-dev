//! CloseShift command.

use serde_json::Value;

use crate::domain::shift::commands::payload::{read_balances, read_request_id, read_text};
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::shift_error::ShiftError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseShift {
    pub request_id: String,
    pub shift_id: String,
    pub counted: Balances,
    pub claimed_expected: Value,
    pub note: String,
}

impl CloseShift {
    pub fn from_payload(p: &Value) -> Result<Self, ShiftError> {
        let request_id = read_request_id(p, "requestId")?;
        let counted = read_balances(p, "counted")?;
        let shift_id = read_text(p, "shiftId", 60, true)?;
        let note = read_text(p, "note", 300, false)?;
        let claimed_expected = p.get("expected").cloned().unwrap_or(Value::Null);

        Ok(Self {
            request_id,
            shift_id,
            counted,
            claimed_expected,
            note,
        })
    }
}
