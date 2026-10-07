//! One line of the device log.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

/// Something that happened at a branch's devices, written once and never changed.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceEvent {
    /// Grows with every line written, over all branches.
    pub seq: i64,
    pub branch_id: String,
    pub actor: String,
    pub kind: String,
    /// What the line is about: a document number, a submission id, a file key, or empty.
    pub reference: String,
    /// What the line says. An amount in it (`foreignAmount` and `thb` on a
    /// `COUNT_CONFIRMED` line) is decimal text — a JSON string holding every
    /// digit — to be read with [`DeviceEvent::amount`], never a JSON number.
    pub detail: Value,
    pub at: DateTime<Utc>,
}

impl DeviceEvent {
    /// The amount the detail holds under `key`, exact; `None` when there is none.
    pub fn amount(&self, key: &str) -> Option<Decimal> {
        self.detail.get(key)?.as_str()?.parse().ok()
    }
}
