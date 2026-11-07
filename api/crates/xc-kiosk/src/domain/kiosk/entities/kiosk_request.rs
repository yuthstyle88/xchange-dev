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
    pub ticket_number: Option<String>,
}

/// The request's queue label: its ticket's `display_number` once a ticket exists, else v2's
/// `K-` and the first eight characters of its id, upper-cased.
pub fn queue_label(id: &str, ticket_number: Option<&str>) -> String {
    if let Some(number) = ticket_number {
        return number.to_owned();
    }
    let first_eight: String = id.chars().take(8).collect();
    format!("K-{}", first_eight.to_uppercase())
}
