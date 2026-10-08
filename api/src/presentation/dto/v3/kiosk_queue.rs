//! Kiosk queue view DTOs for v3 (§7.7, §12.18.2).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::Money;
use crate::presentation::http::v3::view::ViewFilter;

/// The state filter for `kiosk_queue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "kiosk_queue.ts")]
pub enum KioskQueueState {
    Waiting,
    Working,
    Completed,
}

/// Filter for the `kiosk_queue` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "kiosk_queue.ts")]
pub struct KioskQueueFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub state: Option<KioskQueueState>,
}

impl ViewFilter for KioskQueueFilter {}

/// Transaction side in a queue row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export_to = "kiosk_queue.ts")]
pub enum KioskQueueType {
    BUY,
    SELL,
}

/// A row in the `kiosk_queue` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_queue.ts")]
pub struct KioskQueueRow {
    pub id: String,
    pub queue: String,
    pub order_no: String,
    pub transaction_no: Option<String>,
    pub status: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub transaction_type: KioskQueueType,
    pub currency: String,
    pub foreign: Money,
    pub thb: Money,
    pub staff: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    #[ts(type = "number | null")]
    pub duration_seconds: Option<i64>,
    pub can_accept: bool,
    pub can_continue: bool,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    KioskQueueState::export_all(&cfg)?;
    KioskQueueFilter::export_all(&cfg)?;
    KioskQueueType::export_all(&cfg)?;
    KioskQueueRow::export_all(&cfg)?;
    Ok(())
}
