#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KioskStatus {
    Quoted,
    Waiting,
    Accepted,
    IdChecked,
    Completed,
    Expired,
}

impl KioskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quoted => "QUOTED",
            Self::Waiting => "WAITING",
            Self::Accepted => "ACCEPTED",
            Self::IdChecked => "ID_CHECKED",
            Self::Completed => "COMPLETED",
            Self::Expired => "EXPIRED",
        }
    }
}
