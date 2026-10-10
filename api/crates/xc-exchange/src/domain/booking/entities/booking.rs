//! Booking entity.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use crate::domain::booking::value_objects::booking_status::BookingStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Booking {
    pub id: String,
    pub branch_id: BranchId,
    pub name: String,
    pub phone: String,
    pub member_id: Option<String>,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: Money,
    pub rate: Rate,
    pub unit_size: UnitSize,
    pub published_rate_line_id: Option<Uuid>,
    pub status: BookingStatus,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub confirmed_by: Option<String>,
    pub submission_id: Option<String>,
    pub transaction_ref: Option<String>,
}

impl Booking {
    pub fn shown_status(&self, now: DateTime<Utc>) -> &'static str {
        if self.status == BookingStatus::Confirmed {
            if let Some(expires_at) = self.expires_at {
                if expires_at <= now {
                    return "EXPIRED";
                }
            }
        }
        self.status.as_str()
    }

    pub fn is_member(&self) -> bool {
        self.member_id.is_some()
    }
}
