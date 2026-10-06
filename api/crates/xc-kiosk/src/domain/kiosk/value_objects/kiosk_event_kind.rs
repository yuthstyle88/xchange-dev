#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KioskEventKind {
    Request,
    Accepted,
    IdChecked,
    Completed,
}

impl KioskEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "KIOSK_REQUEST",
            Self::Accepted => "KIOSK_ACCEPTED",
            Self::IdChecked => "KIOSK_ID_CHECKED",
            Self::Completed => "KIOSK_COMPLETED",
        }
    }
}
