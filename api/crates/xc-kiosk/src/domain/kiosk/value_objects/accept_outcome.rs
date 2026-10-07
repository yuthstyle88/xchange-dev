use crate::domain::kiosk::value_objects::work_times::WorkTimes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOutcome {
    Claimed(WorkTimes),
    AlreadyMine(WorkTimes),
    HeldBy(String),
    Unavailable,
}
