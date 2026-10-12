//! `BookingPolicy` (design §5, `booking_policies`): the cancellation cutoff, the no-show grace and the confirmation
//! expiry of an organisation. Every number is the row's (B11); none is in code.

use chrono::{DateTime, Utc};

use crate::domain::slot::booking_slot::BookingSlot;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookingPolicy {
    pub cancel_cutoff_minutes: i32,
    pub no_show_grace_minutes: i32,
    pub confirm_expiry_minutes: i32,
}

impl BookingPolicy {
    /// Q-B2: `slot.start_at − cancel_cutoff_minutes`.
    pub fn cutoff_at(&self, slot: &BookingSlot) -> DateTime<Utc> {
        slot.start_at - chrono::Duration::minutes(self.cancel_cutoff_minutes as i64)
    }

    /// Q-B3: `slot.start_at + no_show_grace_minutes`.
    pub fn grace_until(&self, slot: &BookingSlot) -> DateTime<Utc> {
        slot.start_at + chrono::Duration::minutes(self.no_show_grace_minutes as i64)
    }

    /// Q-B4: `now + confirm_expiry_minutes`, never later than [`BookingPolicy::cutoff_at`].
    pub fn confirm_expires_at(&self, now: DateTime<Utc>, slot: &BookingSlot) -> DateTime<Utc> {
        let expiry = now + chrono::Duration::minutes(self.confirm_expiry_minutes as i64);
        let cutoff = self.cutoff_at(slot);
        expiry.min(cutoff)
    }
}
