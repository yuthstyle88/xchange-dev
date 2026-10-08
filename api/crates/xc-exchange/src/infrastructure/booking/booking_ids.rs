//! Booking and member ID generation.

use uuid::Uuid;

use crate::domain::booking::repositories::booking_ids::BookingIds;

pub struct UuidBookingIds;

impl BookingIds for UuidBookingIds {
    fn booking_id(&self) -> String {
        format!("BK-{}", Uuid::new_v4())
    }

    fn member_id(&self) -> String {
        format!("MB-{}", Uuid::new_v4())
    }
}
