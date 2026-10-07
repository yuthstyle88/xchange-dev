use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkTimes {
    pub started_at: Option<KioskTime>,
    pub completed_at: Option<KioskTime>,
}

impl WorkTimes {
    pub fn duration_seconds(&self) -> Option<i64> {
        match (self.started_at, self.completed_at) {
            (Some(started), Some(completed)) => {
                let diff = completed.epoch_millis() - started.epoch_millis();
                Some((diff / 1000).max(0))
            }
            _ => None,
        }
    }
}
