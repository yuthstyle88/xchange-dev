#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KioskAction {
    Rates,
    Quote,
    Confirm,
    Status,
    CustomerReceipt,
    Notifications,
    List,
    Accept,
    Get,
    IdentityChecked,
    PublishReceipt,
    Manage,
    Configure,
    Cancel,
    CustomerCancel,
}

impl KioskAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rates => "rates",
            Self::Quote => "quote",
            Self::Confirm => "confirm",
            Self::Status => "status",
            Self::CustomerReceipt => "customerReceipt",
            Self::Notifications => "notifications",
            Self::List => "list",
            Self::Accept => "accept",
            Self::Get => "get",
            Self::IdentityChecked => "identityChecked",
            Self::PublishReceipt => "publishReceipt",
            Self::Manage => "manage",
            Self::Configure => "configure",
            Self::Cancel => "cancel",
            Self::CustomerCancel => "customerCancel",
        }
    }
}
