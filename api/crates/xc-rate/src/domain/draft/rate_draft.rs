//! Rate draft domain entity and value objects.

use std::fmt;
use std::str::FromStr;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Rate};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DraftId(Uuid);

impl DraftId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for DraftId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for DraftId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStatus {
    Draft,
    PendingApproval,
    Approved,
    Rejected,
    Published,
}

impl DraftStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::PendingApproval => "PENDING_APPROVAL",
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
            Self::Published => "PUBLISHED",
        }
    }

    pub fn parse_db(s: &str) -> Option<Self> {
        match s {
            "DRAFT" => Some(Self::Draft),
            "PENDING_APPROVAL" => Some(Self::PendingApproval),
            "APPROVED" => Some(Self::Approved),
            "REJECTED" => Some(Self::Rejected),
            "PUBLISHED" => Some(Self::Published),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LineSide {
    Auto,
    Manual(Rate),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DraftLine {
    pub currency: CurrencyCode,
    pub buy: LineSide,
    pub sell: LineSide,
    pub reason: Option<String>,
}

impl DraftLine {
    pub fn new(
        currency: CurrencyCode,
        buy: LineSide,
        sell: LineSide,
        reason: Option<String>,
    ) -> Self {
        Self {
            currency,
            buy,
            sell,
            reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RateDraft {
    pub id: DraftId,
    pub status: DraftStatus,
    pub version: i32,
    pub lines: Vec<DraftLine>,
}
