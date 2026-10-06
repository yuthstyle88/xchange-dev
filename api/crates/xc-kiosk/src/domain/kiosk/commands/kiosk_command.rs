#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KioskCommand {
    Confirm,
    Accept,
    IdentityChecked { confirmed: bool },
}
