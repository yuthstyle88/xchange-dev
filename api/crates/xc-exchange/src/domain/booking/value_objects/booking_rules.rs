//! Booking rules value object.

use serde_json::Value;

use super::booking_error::BookingError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookingRules {
    pub enabled: bool,
    pub minutes: u16,
}

impl BookingRules {
    pub const DEFAULT: Self = Self {
        enabled: true,
        minutes: 30,
    };

    pub fn parse(enabled: &Value, minutes: &Value) -> Result<Self, BookingError> {
        let enabled_bool = enabled.as_bool().ok_or(BookingError::BadRules)?;
        let minutes_num = minutes.as_u64().ok_or(BookingError::BadRules)?;
        if !(5..=1440).contains(&minutes_num) {
            return Err(BookingError::BadRules);
        }
        Ok(Self {
            enabled: enabled_bool,
            minutes: minutes_num as u16,
        })
    }
}
