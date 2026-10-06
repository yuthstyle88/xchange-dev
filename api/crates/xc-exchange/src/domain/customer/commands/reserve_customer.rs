//! ReserveCustomer command.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveCustomer {
    pub name: String,
    pub passport_no: String,
    pub nationality: String,
    pub birth_date: String,
    pub passport_expiry: String,
    pub mode: String,
}
