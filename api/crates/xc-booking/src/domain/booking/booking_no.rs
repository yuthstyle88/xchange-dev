//! `BookingNo` (design §4.1, §5): B<yyMMdd>-<0001> formatted booking sequence number.

use chrono::NaiveDate;

pub struct BookingNo;

impl BookingNo {
    /// Formats the sequence number for a booking issued on `date`.
    pub fn format(date: NaiveDate, seq: i32) -> String {
        format!("B{}-{:04}", date.format("%y%m%d"), seq)
    }
}
