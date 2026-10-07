//! Simulated device driver.

use super::device_job::{JobKind, JobStatus};

/// What carries a queued job out.
pub trait DeviceDriver: Send + Sync {
    /// The word the log and the screen show for this driver.
    fn mode(&self) -> &'static str;

    /// Carries the job out and says where it stands afterwards.
    fn run(&self, kind: JobKind, printer_online: bool) -> JobStatus;
}

/// Touches no hardware: it only says what a device would have done.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimulatedDriver;

impl DeviceDriver for SimulatedDriver {
    fn mode(&self) -> &'static str {
        "SIMULATION"
    }

    fn run(&self, kind: JobKind, printer_online: bool) -> JobStatus {
        if !printer_online {
            return JobStatus::Failed;
        }
        match kind {
            JobKind::PrintReceipt => JobStatus::SimulatedPrinted,
            JobKind::DrawerRequest => JobStatus::CommandSimulated,
        }
    }
}
