use crate::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::transaction_link::TransactionLink;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KioskRequest {
    pub id: String,
    pub status: KioskStatus,
    pub created: KioskTime,
    pub staff: Option<String>,
    pub quote: KioskQuote,
    pub transaction: Option<TransactionLink>,
}
