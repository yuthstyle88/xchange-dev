//! `BookingEvent` (design §4.1, §5): append-only audit record of booking lifecycle transitions.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::booking_status::BookingStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BookingEventKind {
    Create,
    Confirm,
    Reschedule,
    CheckIn,
    Start,
    Complete,
    Cancel,
    NoShow,
    Expire,
    QrReissue,
}

impl BookingEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "CREATE",
            Self::Confirm => "CONFIRM",
            Self::Reschedule => "RESCHEDULE",
            Self::CheckIn => "CHECK_IN",
            Self::Start => "START",
            Self::Complete => "COMPLETE",
            Self::Cancel => "CANCEL",
            Self::NoShow => "NO_SHOW",
            Self::Expire => "EXPIRE",
            Self::QrReissue => "QR_REISSUE",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "CREATE" => Some(Self::Create),
            "CONFIRM" => Some(Self::Confirm),
            "RESCHEDULE" => Some(Self::Reschedule),
            "CHECK_IN" => Some(Self::CheckIn),
            "START" => Some(Self::Start),
            "COMPLETE" => Some(Self::Complete),
            "CANCEL" => Some(Self::Cancel),
            "NO_SHOW" => Some(Self::NoShow),
            "EXPIRE" => Some(Self::Expire),
            "QR_REISSUE" => Some(Self::QrReissue),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookingEvent {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub booking_id: String,
    pub kind: BookingEventKind,
    pub from_status: Option<BookingStatus>,
    pub to_status: BookingStatus,
    pub actor_id: String,
    pub at: DateTime<Utc>,
    pub reason: Option<String>,
    pub detail: Value,
    pub created_at: DateTime<Utc>,
}
