use crate::domain::kiosk::value_objects::kiosk_role::KioskRole;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KioskActor {
    Customer,
    Staff { username: String, role: KioskRole },
}
