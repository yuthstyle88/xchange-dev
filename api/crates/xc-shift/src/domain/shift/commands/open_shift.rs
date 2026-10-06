//! OpenShift command.

use serde_json::Value;

use crate::domain::shift::commands::payload::{read_balances, read_request_id};
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::shift_error::ShiftError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenShift {
    pub request_id: String,
    pub opening: Balances,
}

impl OpenShift {
    pub fn from_payload(p: &Value) -> Result<Self, ShiftError> {
        let request_id = read_request_id(p, "requestId")?;
        let opening = read_balances(p, "opening")?;
        Ok(Self {
            request_id,
            opening,
        })
    }
}
