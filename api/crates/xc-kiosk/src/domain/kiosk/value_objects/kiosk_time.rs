#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KioskTime(i64);

impl KioskTime {
    pub fn from_epoch_millis(millis: i64) -> Self {
        Self(millis)
    }

    pub fn epoch_millis(self) -> i64 {
        self.0
    }
}
