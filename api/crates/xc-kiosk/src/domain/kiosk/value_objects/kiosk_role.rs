#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KioskRole {
    Owner,
    Front,
    Backend,
    Manager,
    Rates,
    Other,
}

impl KioskRole {
    pub fn is_desk(self) -> bool {
        matches!(
            self,
            Self::Owner | Self::Front | Self::Backend | Self::Manager
        )
    }
}
