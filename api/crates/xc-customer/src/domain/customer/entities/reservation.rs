//! Reservation entity.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    pub customer_id: String,
    pub existing: bool,
}
