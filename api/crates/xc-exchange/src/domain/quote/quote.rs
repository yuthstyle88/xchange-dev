//! Quote domain entity and value objects.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

use super::quote_line::QuoteLine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStatus {
    Active,
    Accepted,
    Cancelled,
}

impl QuoteStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Accepted => "ACCEPTED",
            Self::Cancelled => "CANCELLED",
        }
    }

    pub fn parse_db(s: &str) -> Option<Self> {
        match s {
            "ACTIVE" => Some(Self::Active),
            "ACCEPTED" => Some(Self::Accepted),
            "CANCELLED" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteDisplayStatus {
    Active,
    Accepted,
    Cancelled,
    Expired,
}

impl QuoteDisplayStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Accepted => "ACCEPTED",
            Self::Cancelled => "CANCELLED",
            Self::Expired => "EXPIRED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteChannel {
    Desk,
    Kiosk,
    Booking,
    App,
}

impl QuoteChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Desk => "DESK",
            Self::Kiosk => "KIOSK",
            Self::Booking => "BOOKING",
            Self::App => "APP",
        }
    }

    pub fn parse_db(s: &str) -> Option<Self> {
        match s {
            "DESK" => Some(Self::Desk),
            "KIOSK" => Some(Self::Kiosk),
            "BOOKING" => Some(Self::Booking),
            "APP" => Some(Self::App),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub customer_id: Option<String>,
    pub quote_type: String,
    pub channel: QuoteChannel,
    pub status: QuoteStatus,
    pub publication_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub benefit_snapshot: serde_json::Value,
    pub created_by: String,
    pub accepted_by_transaction_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lines: Vec<QuoteLine>,
    pub stale: bool,
}

impl Quote {
    pub fn is_valid(&self, now: DateTime<Utc>, current_publication: Uuid) -> bool {
        now < self.expires_at && self.publication_id == current_publication
    }

    pub fn status_at(&self, now: DateTime<Utc>, current_publication: Uuid) -> QuoteDisplayStatus {
        match self.status {
            QuoteStatus::Accepted => QuoteDisplayStatus::Accepted,
            QuoteStatus::Cancelled => QuoteDisplayStatus::Cancelled,
            QuoteStatus::Active => {
                if self.is_valid(now, current_publication) {
                    QuoteDisplayStatus::Active
                } else {
                    QuoteDisplayStatus::Expired
                }
            }
        }
    }
}
