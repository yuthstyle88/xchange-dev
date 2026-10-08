//! Request input and response DTOs for `POST /api/drawer-passport` (design §5.5.5, §5.5.8).

use serde_json::Value;

use crate::application::workflows::passport_drawer_device::PassportDrawerState;

/// Input payload of `POST /api/drawer-passport` (`desk-passport-drawer.js:13`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PassportDrawerInput {
    pub order_id: Option<String>,
    pub kiosk: bool,
    pub mode: Option<String>,
}

impl PassportDrawerInput {
    pub fn from_value(value: &Value) -> Self {
        let order_id = value
            .get("orderId")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let kiosk = value.get("kiosk").and_then(Value::as_bool).unwrap_or(false);
        let mode = value
            .get("mode")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        Self {
            order_id,
            kiosk,
            mode,
        }
    }
}

/// The response body of `POST /api/drawer-passport`:
/// `{"state": <word>, "physicalResultConfirmed": false}`, plus `"duplicate": true` only for row 11.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassportDrawerAnswer {
    pub state: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
    pub physical_result_confirmed: bool,
}

impl PassportDrawerAnswer {
    pub fn new(state: PassportDrawerState) -> Self {
        Self {
            state: state.as_str(),
            duplicate: false,
            physical_result_confirmed: false,
        }
    }

    pub fn with_duplicate(state: PassportDrawerState, duplicate: bool) -> Self {
        Self {
            state: state.as_str(),
            duplicate,
            physical_result_confirmed: false,
        }
    }
}

pub fn drawer_passport_json(state: PassportDrawerState) -> serde_json::Value {
    serde_json::json!({
        "state": state.as_str(),
        "physicalResultConfirmed": false,
    })
}
