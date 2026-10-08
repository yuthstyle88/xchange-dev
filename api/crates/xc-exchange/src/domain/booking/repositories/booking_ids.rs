//! Booking and member ID generation trait.

pub trait BookingIds: Send + Sync {
    fn booking_id(&self) -> String;
    fn member_id(&self) -> String;
}
