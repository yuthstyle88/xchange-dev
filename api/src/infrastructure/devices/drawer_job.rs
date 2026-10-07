//! `DrawerJobKey` and `DrawerJobOutcome`, filled by row AG-3 (design §5.4c).

use xc_kernel::ids::BranchId;

use super::device_job::DeviceJob;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassportSource {
    Kiosk,
    Desk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawerJobKey {
    CashSave {
        reference: String,
    },
    Passport {
        source: PassportSource,
        order_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawerJobOutcome {
    Queued(DeviceJob),
    Existing(DeviceJob),
    NoDrawer,
    NoAgent,
}

impl DrawerJobKey {
    /// Builds the request id: `auto:<branch>:drawer:<reference>` or
    /// `auto:<branch>:passport:<KIOSK|DESK>:<order_id>`.
    pub fn request_id(&self, branch: &BranchId) -> String {
        match self {
            Self::CashSave { reference } => format!("auto:{}:drawer:{reference}", branch.as_str()),
            Self::Passport { source, order_id } => {
                let source_str = match source {
                    PassportSource::Kiosk => "KIOSK",
                    PassportSource::Desk => "DESK",
                };
                format!("auto:{}:passport:{source_str}:{order_id}", branch.as_str())
            }
        }
    }

    /// Builds the fixed reason: `CASH_EXCHANGE` or `PASSPORT_STEP`.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::CashSave { .. } => "CASH_EXCHANGE",
            Self::Passport { .. } => "PASSPORT_STEP",
        }
    }

    /// Builds the job's `ref`: the document ref for `CashSave`, `""` for `Passport`.
    pub fn reference(&self) -> &str {
        match self {
            Self::CashSave { reference } => reference.as_str(),
            Self::Passport { .. } => "",
        }
    }
}
