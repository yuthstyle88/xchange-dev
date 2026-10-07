//! Simulated device driver.

use async_trait::async_trait;

use super::device_job::{JobKind, JobStatus};
use super::printer_models::DrawerSettings;
use super::printer_settings::PrinterSettings;

/// What carries a queued job out.
#[async_trait]
pub trait DeviceDriver: Send + Sync {
    /// The word the log and the screen show for this driver.
    fn mode(&self) -> &'static str;

    /// Carries the job out on the branch's printer and says where it stands afterwards.
    async fn run(&self, kind: JobKind, printer: &PrinterSettings) -> JobStatus;

    /// Carries the job out on the branch's printer and drawer settings and says where it stands afterwards.
    async fn run_with(
        &self,
        kind: JobKind,
        printer: &PrinterSettings,
        drawer: &DrawerSettings,
    ) -> JobStatus {
        let _ = drawer;
        self.run(kind, printer).await
    }
}

/// Touches no hardware: it only says what a device would have done.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimulatedDriver;

#[async_trait]
impl DeviceDriver for SimulatedDriver {
    fn mode(&self) -> &'static str {
        "SIMULATION"
    }

    async fn run(&self, kind: JobKind, printer: &PrinterSettings) -> JobStatus {
        if !printer.online {
            return JobStatus::Failed;
        }
        match kind {
            JobKind::PrintReceipt => JobStatus::SimulatedPrinted,
            JobKind::DrawerRequest => JobStatus::CommandSimulated,
        }
    }
}
