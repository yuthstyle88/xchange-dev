//! Passport drawer workflow orchestrating drawer opening checks (design §5.5.5, §5.5.8).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use xc_kernel::ids::BranchId;

use crate::application::workflows::passport_drawer_device::{
    PassportDrawerDevice, PassportDrawerOutcome, PassportDrawerState,
};
use crate::infrastructure::devices::drawer_job::PassportSource;
use crate::presentation::dto::drawer_passport::PassportDrawerInput;

/// Domain refusal reasons for passport drawer requests.
#[derive(Debug, thiserror::Error)]
pub enum PassportDrawerRefusal {
    #[error("สาขาไม่ถูกต้อง")]
    InvalidBranch,
    #[error("รหัสออเดอร์ไม่ถูกต้อง")]
    InvalidOrderId,
    #[error("ต้องเป็นผู้รับเรื่องของคิวนี้ก่อนเปิดลิ้นชัก")]
    KioskNotHeld,
    #[error("ออเดอร์นี้เป็นของพนักงานคนอื่น")]
    OrderHeldByOther,
    #[error(transparent)]
    Database(#[from] pos_error::AppError),
}

/// Reply carrying the state and whether this ask is a repeat of an existing job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassportDrawerReply {
    pub state: PassportDrawerState,
    pub duplicate: bool,
}

fn is_valid_order_id(order_id: &str) -> bool {
    order_id.len() == 36
        && order_id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
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
    ) -> Result<PassportDrawerReply, PassportDrawerRefusal> {
        // Row 5: branch shape check
        if !BranchId::is_well_formed(branch.as_str()) {
            return Err(PassportDrawerRefusal::InvalidBranch);
        }

        // Row 6: mode ≠ "LIVE" (absent included)
        if input.mode.as_deref() != Some("LIVE") {
            return Ok(PassportDrawerReply {
                state: PassportDrawerState::Simulated,
                duplicate: false,
            });
        }

        // Row 7: Switched off (no device, or !device.switched_on(branch))
        let Some(device) = &self.device else {
            return Ok(PassportDrawerReply {
                state: PassportDrawerState::Disabled,
                duplicate: false,
            });
        };
        if !device.switched_on(branch) {
            return Ok(PassportDrawerReply {
                state: PassportDrawerState::Disabled,
                duplicate: false,
            });
        }

        // Row 8: order id check (must be 36 characters of [a-f0-9-])
        let order_id = match input.order_id.as_deref() {
            Some(id) if is_valid_order_id(id) => id,
            _ => return Err(PassportDrawerRefusal::InvalidOrderId),
        };

        // Rows 9-12: With device (AG-PD)
        let source = if input.kiosk {
            PassportSource::Kiosk
        } else {
            PassportSource::Desk
        };

        let outcome = device.request(branch, source, order_id, actor, now).await?;
        match outcome {
            PassportDrawerOutcome::State { state, duplicate } => {
                Ok(PassportDrawerReply { state, duplicate })
            }
            PassportDrawerOutcome::NotHolder => Err(PassportDrawerRefusal::KioskNotHeld),
            PassportDrawerOutcome::OtherEmployee => Err(PassportDrawerRefusal::OrderHeldByOther),
        }
    }
}
