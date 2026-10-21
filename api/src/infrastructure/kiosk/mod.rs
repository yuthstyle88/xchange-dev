//! What the kiosk needs from the other contexts (ticket #172, T12c).
pub mod kiosk_ports;
pub mod pg_kiosk_settings_repository;

pub use kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
