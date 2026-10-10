//! `OutboxEvent` (design §5.1, Blueprint §07 `OutboxEvent`): one row of `outbox_events` as the
//! worker hands it to a handler.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// `outbox_events.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatus {
    Pending,
    Delivered,
    Failed,
    Expired,
}

impl OutboxStatus {
    /// The stored text: `PENDING`, `DELIVERED`, `FAILED`, `EXPIRED`.
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxStatus::Pending => "PENDING",
            OutboxStatus::Delivered => "DELIVERED",
            OutboxStatus::Failed => "FAILED",
            OutboxStatus::Expired => "EXPIRED",
        }
    }

    /// The inverse of `as_str`; `None` for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PENDING" => Some(OutboxStatus::Pending),
            "DELIVERED" => Some(OutboxStatus::Delivered),
            "FAILED" => Some(OutboxStatus::Failed),
            "EXPIRED" => Some(OutboxStatus::Expired),
            _ => None,
        }
    }
}

/// One outbox row. `payload_redacted` holds references (ids), never personal data; the handler
/// rebuilds its wire body from the source rows when it sends.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEvent {
    /// `outbox_events.id` as text.
    pub id: String,
    /// Enqueue order; FIFO per aggregate and destination follows it.
    pub seq: i64,
    /// `outbox_events.organization_id` (a uuid) as text.
    pub organization_id: String,
    pub event_type: String,
    pub destination: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    /// Sent with every delivery; the receiver dedupes on it.
    pub idempotency_key: String,
    pub payload_redacted: Value,
    pub status: OutboxStatus,
    /// Delivery attempts so far, this one included (the claim adds 1).
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
}
