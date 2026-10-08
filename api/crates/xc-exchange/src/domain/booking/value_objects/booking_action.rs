//! Booking action value object.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookingAction {
    List,
    Get,
    Create,
    Confirm,
    Cancel,
    Member,
    Settings,
    Registrations,
    ApproveRegistration,
}

impl BookingAction {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "list" => Some(Self::List),
            "get" => Some(Self::Get),
            "create" => Some(Self::Create),
            "confirm" => Some(Self::Confirm),
            "cancel" => Some(Self::Cancel),
            "member" => Some(Self::Member),
            "settings" => Some(Self::Settings),
            "registrations" => Some(Self::Registrations),
            "approveRegistration" => Some(Self::ApproveRegistration),
            _ => None,
        }
    }

    pub const fn owner_only(self) -> bool {
        matches!(
            self,
            Self::Member | Self::Settings | Self::Registrations | Self::ApproveRegistration
        )
    }
}
