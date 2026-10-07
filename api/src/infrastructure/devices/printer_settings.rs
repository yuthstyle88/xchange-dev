//! The printer a branch has, as its settings say.

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::device_repository::{DeviceRepository, PRINTER_ONLINE};

/// The setting that says how the branch's printer is reached.
pub const PRINTER_KIND: &str = "printerKind";
/// The host of a network printer.
pub const PRINTER_HOST: &str = "printerHost";
/// The TCP port of a network printer.
pub const PRINTER_PORT: &str = "printerPort";
/// The port a network printer listens on when the branch names none.
pub const DEFAULT_PORT: u16 = 9100;

/// How the branch's printer is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrinterKind {
    /// No hardware: the driver only says what a device would have done.
    Simulated,
    /// A printer the server reaches over TCP: host and port.
    Network(String, u16),
}

/// The branch's printer: the switch and how it is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterSettings {
    pub online: bool,
    pub kind: PrinterKind,
}

impl PrinterSettings {
    /// The branch's printer as the transaction sees its settings.
    pub async fn read_in<R: DeviceRepository>(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        repo: &R,
        branch: &BranchId,
    ) -> AppResult<Self> {
        let online = repo
            .setting_in(tx, branch, PRINTER_ONLINE)
            .await?
            .as_deref()
            != Some("0");

        let kind_str = repo.setting_in(tx, branch, PRINTER_KIND).await?;
        let kind = match kind_str.as_deref() {
            None | Some("SIMULATED") => PrinterKind::Simulated,
            Some("NETWORK") => {
                let host_str = repo.setting_in(tx, branch, PRINTER_HOST).await?;
                let host = match host_str {
                    Some(h) if !h.trim().is_empty() => h,
                    _ => {
                        return Err(pos_error::AppError::Internal(format!(
                            "{PRINTER_HOST} cannot be used"
                        )));
                    }
                };

                let port = match repo.setting_in(tx, branch, PRINTER_PORT).await? {
                    None => DEFAULT_PORT,
                    Some(p_str) => match p_str.parse::<u16>() {
                        Ok(p) if p > 0 => p,
                        _ => {
                            return Err(pos_error::AppError::Internal(format!(
                                "{PRINTER_PORT} cannot be used"
                            )));
                        }
                    },
                };

                PrinterKind::Network(host, port)
            }
            Some(other) => {
                return Err(pos_error::AppError::Internal(format!(
                    "{PRINTER_KIND}: unknown kind {other}"
                )));
            }
        };

        Ok(Self { online, kind })
    }
}
