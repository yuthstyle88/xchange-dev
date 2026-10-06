//! Custody time representation in epoch milliseconds.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustodyTime(i64);

impl CustodyTime {
    pub fn from_epoch_millis(millis: i64) -> CustodyTime {
        CustodyTime(millis)
    }

    pub fn epoch_millis(self) -> i64 {
        self.0
    }

    /// Minutes after midnight in Bangkok (UTC+7, no daylight saving): 11:00 is 660.
    pub fn bangkok_minute_of_day(self) -> u16 {
        let bkk_millis = self.0 + 7 * 60 * 60 * 1000;
        let minute = bkk_millis.div_euclid(60 * 1000).rem_euclid(1440);
        minute as u16
    }
}
