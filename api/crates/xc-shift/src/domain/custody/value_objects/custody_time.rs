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
}
