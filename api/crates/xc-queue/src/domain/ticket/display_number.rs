//! `DisplayNumber` (design §5, Q3): the type's letter and the sequence zero-padded to 3.

use super::service_type::ServiceType;

/// The number shown to the customer and called at the desk.
pub struct DisplayNumber;

impl DisplayNumber {
    /// `W001`, `B012`, `W999`; a sequence above 999 keeps all its digits (`W1000`).
    pub fn format(service_type: ServiceType, sequence: i32) -> String {
        format!("{}{:03}", service_type.letter(), sequence)
    }
}
