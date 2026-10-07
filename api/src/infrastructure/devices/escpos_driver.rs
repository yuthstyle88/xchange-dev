//! A driver that reaches a real ESC/POS printer over the network.

use std::sync::Arc;

use async_trait::async_trait;

use super::device_job::{JobKind, JobStatus};
use super::printer_models::{DrawerSettings, PrinterModels};
use super::printer_settings::{PrinterKind, PrinterSettings};
use super::simulated_driver::DeviceDriver;

/// Carries bytes to a printer.
#[async_trait]
pub trait PrinterTransport: Send + Sync {
    async fn send(&self, host: &str, port: u16, bytes: &[u8]) -> anyhow::Result<()>;
}

/// A raw TCP write to `host:port`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TcpTransport;

#[async_trait]
impl PrinterTransport for TcpTransport {
    async fn send(&self, host: &str, port: u16, bytes: &[u8]) -> anyhow::Result<()> {
        pos_printer::network_printer::send(host, port, bytes).await?;
        Ok(())
    }
}

/// Answers a simulated printer as `SimulatedDriver` does and drives a network one.
pub struct EscposDriver {
    transport: Arc<dyn PrinterTransport>,
    models: Arc<PrinterModels>,
}

impl EscposDriver {
    pub fn new(transport: Arc<dyn PrinterTransport>) -> Self {
        Self::with_models(transport, Arc::new(PrinterModels::built_in()))
    }

    pub fn with_models(transport: Arc<dyn PrinterTransport>, models: Arc<PrinterModels>) -> Self {
        Self { transport, models }
    }
}

#[async_trait]
impl DeviceDriver for EscposDriver {
    fn mode(&self) -> &'static str {
        "SIMULATION"
    }

    async fn run(&self, kind: JobKind, printer: &PrinterSettings) -> JobStatus {
        let drawer = DrawerSettings {
            model: self.models.default_id().to_owned(),
            has_drawer: None,
        };
        self.run_with(kind, printer, &drawer).await
    }

    async fn run_with(
        &self,
        kind: JobKind,
        printer: &PrinterSettings,
        drawer: &DrawerSettings,
    ) -> JobStatus {
        if !printer.online {
            return JobStatus::Failed;
        }

        match &printer.kind {
            PrinterKind::Simulated => match kind {
                JobKind::PrintReceipt => JobStatus::SimulatedPrinted,
                JobKind::DrawerRequest => JobStatus::CommandSimulated,
            },
            PrinterKind::Network(host, port) => match kind {
                JobKind::PrintReceipt => JobStatus::SimulatedPrinted,
                JobKind::DrawerRequest => {
                    if drawer.has_drawer == Some(false) {
                        return JobStatus::Failed;
                    }
                    let Some(model) = self.models.get(&drawer.model) else {
                        return JobStatus::Failed;
                    };
                    let Some(pulse) = &model.drawer_pulse else {
                        return JobStatus::Failed;
                    };
                    let kick = pulse.kick_bytes();
                    match self.transport.send(host, *port, &kick).await {
                        Ok(()) => JobStatus::DrawerOpened,
                        Err(_) => JobStatus::Failed,
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_network_printer_with_switch_off_fails_without_sending() {
        struct FailingTransport;
        #[async_trait]
        impl PrinterTransport for FailingTransport {
            async fn send(&self, _host: &str, _port: u16, _bytes: &[u8]) -> anyhow::Result<()> {
                panic!("should not send when printer is offline");
            }
        }

        let driver = EscposDriver::new(Arc::new(FailingTransport));
        let settings = PrinterSettings {
            online: false,
            kind: PrinterKind::Network("127.0.0.1".to_string(), 9100),
        };
        let status = driver.run(JobKind::DrawerRequest, &settings).await;
        assert_eq!(status, JobStatus::Failed);
    }
}
