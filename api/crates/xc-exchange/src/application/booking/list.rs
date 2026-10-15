//! Booking reads (list, get).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;

use crate::domain::booking::entities::booking::Booking;
use crate::domain::booking::entities::member::Member;
use crate::domain::booking::value_objects::booking_error::BookingError;
use crate::domain::booking::value_objects::booking_rules::BookingRules;
use xc_customer::application::customer::service::NO_SUCH_BRANCH;

use super::service::BookingService;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookingList {
    pub bookings: Vec<Booking>,
    pub members: Vec<Member>,
    pub rules: Option<BookingRules>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookingView {
    pub booking: Booking,
    pub customer_id: Option<String>,
}

impl BookingService {
    pub async fn list(&self, branch: &BranchId, _now: DateTime<Utc>) -> AppResult<BookingList> {
        if !self.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        let bookings = self.bookings.list(branch, 1001).await?;
        if bookings.len() > 1000 {
            return Err(AppError::Domain(BookingError::HistoryTooLong.to_string()));
        }

        let members = self.members.list(branch).await?;
        let rules = self.ports.rules(branch).await?;

        Ok(BookingList {
            bookings,
            members,
            rules,
        })
    }

    pub async fn get(&self, branch: &BranchId, id: &str) -> AppResult<BookingView> {
        if !self.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        let booking = self
            .bookings
            .find(branch, id)
            .await?
            .ok_or_else(|| AppError::Domain(BookingError::NotFound.to_string()))?;

        let customer_id = if let Some(ref member_id) = booking.member_id {
            let member = self.members.find(branch, member_id).await?;
            member.map(|m| m.customer_id)
        } else {
            None
        };

        Ok(BookingView {
            booking,
            customer_id,
        })
    }
}
