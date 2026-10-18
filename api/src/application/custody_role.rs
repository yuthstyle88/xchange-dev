//! The one mapping from an employee's stored role text to its part in custody.

use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;

/// The custody role of an employee's role as stored. A role with no part in
/// custody, and anything unknown, is `Other`; an employee is never `Owner`.
pub fn employee_custody_role(stored: &str) -> CustodyRole {
    match stored {
        "FRONT" => CustodyRole::Front,
        "FINANCE" => CustodyRole::Finance,
        "MONITOR" => CustodyRole::Monitor,
        "MANAGER" => CustodyRole::Manager,
        "BACKEND" => CustodyRole::Backend,
        _ => CustodyRole::Other,
    }
}
