//! `NotificationDelivery` — one row of `notification_deliveries` (design §4.2 states).

use super::channel::Channel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Sent,
    Failed,
    Skipped,
}

impl DeliveryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeliveryStatus::Pending => "PENDING",
            DeliveryStatus::Sent => "SENT",
            DeliveryStatus::Failed => "FAILED",
            DeliveryStatus::Skipped => "SKIPPED",
        }
    }
}

/// `skipped_reason` when a marketing template has no current consent (design §4.1).
pub const NO_CONSENT: &str = "noConsent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationDelivery {
    pub id: String,
    pub notification_id: String,
    pub channel: Channel,
    pub status: DeliveryStatus,
    pub attempt_no: i32,
    pub skipped_reason: Option<String>,
}
