//! `Capture` (design §4.1, Blueprint §07 `ReferralAttribution`): one row of
//! `referral_attributions`.

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureId(Uuid);

impl CaptureId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for CaptureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// `status`: `ACTIVE` or `SUPERSEDED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStatus {
    Active,
    Superseded,
}

impl CaptureStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Superseded => "SUPERSEDED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ACTIVE" => Some(Self::Active),
            "SUPERSEDED" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// What the code was captured on: `bookings.id` or `transactions.id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Booking(String),
    Transaction(Uuid),
}

/// `source`: where the code was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSource {
    App,
    Booking,
    Desk,
}

impl CaptureSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "APP",
            Self::Booking => "BOOKING",
            Self::Desk => "DESK",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "APP" => Some(Self::App),
            "BOOKING" => Some(Self::Booking),
            "DESK" => Some(Self::Desk),
            _ => None,
        }
    }
}

/// One `referral_attributions` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub id: CaptureId,
    pub organization_id: Uuid,
    pub branch_id: String,
    pub code: String,
    pub customer_id: String,
    pub customer_sub: Option<String>,
    pub booking_id: Option<String>,
    pub transaction_id: Option<Uuid>,
    pub source: CaptureSource,
    pub captured_at: DateTime<Utc>,
    pub captured_by: String,
    pub status: CaptureStatus,
    pub supersedes_id: Option<CaptureId>,
    pub reason: Option<String>,
    pub pushed_at: Option<DateTime<Utc>>,
    pub broker_outcome: Option<String>,
}
