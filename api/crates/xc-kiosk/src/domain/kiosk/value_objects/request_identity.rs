use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestIdentity {
    pub id: String,
    pub queue: String,
    pub order_no: String,
    pub transaction_no: Option<String>,
    pub status: KioskStatus,
}
