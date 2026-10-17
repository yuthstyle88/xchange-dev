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
            .all(|b| (b'a'..=b'f').contains(&b) || b.is_ascii_digit() || b == b'-')
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockDevice {
        call_count: AtomicUsize,
    }

    impl MockDevice {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    impl PassportDrawerDevice for MockDevice {
        fn switched_on(&self, _branch: &BranchId) -> bool {
            true
        }

        fn request<'a>(
            &'a self,
            _branch: &'a BranchId,
            _source: PassportSource,
            _order_id: &'a str,
            _actor: &'a str,
            _now: DateTime<Utc>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = pos_error::AppResult<PassportDrawerOutcome>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                Ok(PassportDrawerOutcome::State {
                    state: PassportDrawerState::Requested,
                    duplicate: false,
                })
            })
        }
    }

    fn test_setup() -> (PassportDrawerWorkflow, Arc<MockDevice>, BranchId) {
        let device = Arc::new(MockDevice::new());
        let workflow = PassportDrawerWorkflow::new(Some(device.clone()));
        let branch = BranchId::new("MAIN");
        (workflow, device, branch)
    }

    #[tokio::test]
    async fn row_1_all_z_order_id_refused_and_device_not_called() {
        let (workflow, device, branch) = test_setup();
        let input = PassportDrawerInput {
            order_id: Some("zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz".to_string()),
            kiosk: false,
            mode: Some("LIVE".to_string()),
        };
        let res = workflow.enter(&branch, &input, "emp-1", Utc::now()).await;
        assert!(matches!(res, Err(PassportDrawerRefusal::InvalidOrderId)));
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn row_2_single_g_order_id_refused_and_device_not_called() {
        let (workflow, device, branch) = test_setup();
        let input = PassportDrawerInput {
            order_id: Some("0123456g-89ab-cdef-0123-456789abcdef".to_string()),
            kiosk: false,
            mode: Some("LIVE".to_string()),
        };
        let res = workflow.enter(&branch, &input, "emp-1", Utc::now()).await;
        assert!(matches!(res, Err(PassportDrawerRefusal::InvalidOrderId)));
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn row_3_valid_hex_order_id_passes_to_device() {
        let (workflow, device, branch) = test_setup();
        let input = PassportDrawerInput {
            order_id: Some("01234567-89ab-cdef-0123-456789abcdef".to_string()),
            kiosk: false,
            mode: Some("LIVE".to_string()),
        };
        let res = workflow.enter(&branch, &input, "emp-1", Utc::now()).await;
        assert_eq!(
            res.unwrap(),
            PassportDrawerReply {
                state: PassportDrawerState::Requested,
                duplicate: false,
            }
        );
        assert_eq!(device.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn row_4_uppercase_hex_order_id_refused() {
        let (workflow, device, branch) = test_setup();
        let input = PassportDrawerInput {
            order_id: Some("0123456A-89ab-cdef-0123-456789abcdef".to_string()),
            kiosk: false,
            mode: Some("LIVE".to_string()),
        };
        let res = workflow.enter(&branch, &input, "emp-1", Utc::now()).await;
        assert!(matches!(res, Err(PassportDrawerRefusal::InvalidOrderId)));
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn row_5_wrong_length_order_id_refused() {
        let (workflow, device, branch) = test_setup();
        // 35 characters
        let input_35 = PassportDrawerInput {
            order_id: Some("01234567-89ab-cdef-0123-456789abcde".to_string()),
            kiosk: false,
            mode: Some("LIVE".to_string()),
        };
        let res_35 = workflow
            .enter(&branch, &input_35, "emp-1", Utc::now())
            .await;
        assert!(matches!(res_35, Err(PassportDrawerRefusal::InvalidOrderId)));
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);

        // 37 characters
        let input_37 = PassportDrawerInput {
            order_id: Some("01234567-89ab-cdef-0123-456789abcdef0".to_string()),
            kiosk: false,
            mode: Some("LIVE".to_string()),
        };
        let res_37 = workflow
            .enter(&branch, &input_37, "emp-1", Utc::now())
            .await;
        assert!(matches!(res_37, Err(PassportDrawerRefusal::InvalidOrderId)));
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn row_6_absent_order_id_refused() {
        let (workflow, device, branch) = test_setup();
        let input = PassportDrawerInput {
            order_id: None,
            kiosk: false,
            mode: Some("LIVE".to_string()),
        };
        let res = workflow.enter(&branch, &input, "emp-1", Utc::now()).await;
        assert!(matches!(res, Err(PassportDrawerRefusal::InvalidOrderId)));
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn row_7_mode_not_live_simulated_no_check() {
        let (workflow, device, branch) = test_setup();

        // mode is None
        let input_none = PassportDrawerInput {
            order_id: Some("invalid-order-id-not-checked".to_string()),
            kiosk: false,
            mode: None,
        };
        let res_none = workflow
            .enter(&branch, &input_none, "emp-1", Utc::now())
            .await;
        assert_eq!(
            res_none.unwrap(),
            PassportDrawerReply {
                state: PassportDrawerState::Simulated,
                duplicate: false,
            }
        );
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);

        // mode is SIMULATED
        let input_sim = PassportDrawerInput {
            order_id: Some("invalid-order-id-not-checked".to_string()),
            kiosk: false,
            mode: Some("SIMULATED".to_string()),
        };
        let res_sim = workflow
            .enter(&branch, &input_sim, "emp-1", Utc::now())
            .await;
        assert_eq!(
            res_sim.unwrap(),
            PassportDrawerReply {
                state: PassportDrawerState::Simulated,
                duplicate: false,
            }
        );
        assert_eq!(device.call_count.load(Ordering::SeqCst), 0);
    }
}
