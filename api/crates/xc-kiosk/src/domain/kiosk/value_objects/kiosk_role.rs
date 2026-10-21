#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KioskRole {
    Owner,
    Front,
    Backend,
    Manager,
    Rates,
    Other,
}
