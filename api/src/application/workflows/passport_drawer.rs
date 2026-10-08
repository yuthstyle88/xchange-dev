//! Passport drawer workflow orchestrating drawer opening checks (design §5.5.5, §5.5.8).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use xc_kernel::ids::BranchId;

use crate::application::workflows::passport_drawer_device::{
    PassportDrawerDevice, PassportDrawerState,
};
use crate::presentation::dto::drawer_passport::PassportDrawerInput;

/// Domain refusal reasons for passport drawer requests.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PassportDrawerRefusal {
    #[error("สาขาไม่ถูกต้อง")]
    InvalidBranch,
    #[error("รหัสออเดอร์ไม่ถูกต้อง")]
    InvalidOrderId,
    #[error("ต้องเป็นผู้รับเรื่องของคิวนี้ก่อนเปิดลิ้นชัก")]
    KioskNotHeld,
    #[error("ออเดอร์นี้เป็นของพนักงานคนอื่น")]
    OrderHeldByOther,
}

/// Orchestrates passport drawer opening requests.
#[derive(Clone)]
pub struct PassportDrawerWorkflow {
    device: Option<Arc<dyn PassportDrawerDevice>>,
}

impl PassportDrawerWorkflow {
    pub fn new(device: Option<Arc<dyn PassportDrawerDevice>>) -> Self {
        Self { device }
    }

    pub async fn enter(
        &self,
        branch: &BranchId,
        input: &PassportDrawerInput,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<PassportDrawerState, PassportDrawerRefusal> {
        // Row 5: branch shape check
        if !BranchId::is_well_formed(branch.as_str()) {
            return Err(PassportDrawerRefusal::InvalidBranch);
        }

        // Row 6: mode ≠ "LIVE" (absent included)
        if input.mode.as_deref() != Some("LIVE") {
            return Ok(PassportDrawerState::Simulated);
        }

        // Row 7: Switched off (no device)
        let Some(device) = &self.device else {
            return Ok(PassportDrawerState::Disabled);
        };

        // Rows 8-11: With device (PD-1 / AG-PD)
        let order_key = input.order_id.as_deref().unwrap_or("");
        Ok(device.request(branch, order_key, actor, now).await)
    }
}
