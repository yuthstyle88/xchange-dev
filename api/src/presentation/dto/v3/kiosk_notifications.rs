//! Kiosk notifications view DTOs for v3 (§7.7, §12.18.2).

use std::path::Path;

use serde::Serialize;
use ts_rs::TS;

use crate::presentation::dto::v3::common::MeRole;

/// A working entry in `kiosk_notifications`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_notifications.ts")]
pub struct KioskNotificationsWorking {
    pub id: String,
    pub queue: String,
    pub staff: String,
    pub started_at: Option<String>,
    pub status: String,
}

/// The employee identity in `kiosk_notifications`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_notifications.ts")]
pub struct KioskNotificationsEmployee {
    pub id: String,
    // The caller's role rows, as `me` answers them (#1030 §3.4).
    pub roles: Vec<MeRole>,
}

/// Data payload of the `kiosk_notifications` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_notifications.ts")]
pub struct KioskNotifications {
    pub server_now: String,
    pub branch_id: String,
    pub branch_name: String,
    #[ts(type = "number")]
    pub count: i64,
    pub latest: Option<String>,
    #[ts(type = "number")]
    pub working_count: i64,
    pub working: Vec<KioskNotificationsWorking>,
    pub employee: KioskNotificationsEmployee,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    KioskNotificationsWorking::export_all(&cfg)?;
    KioskNotificationsEmployee::export_all(&cfg)?;
    KioskNotifications::export_all(&cfg)?;
    Ok(())
}
