//! Issue member command.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMember {
    pub customer_id: String,
    pub identity_confirmed: bool,
    pub phone: String,
    pub secret: String,
}
