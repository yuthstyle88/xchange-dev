//! What callers hand the device service.

use rust_decimal::Decimal;

/// A request to queue a job, as the screen sends it: nothing is trimmed or
/// judged before the service sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueJob {
    pub kind: String,
    /// Document number of the exchange; empty for none.
    pub reference: String,
    pub request_id: String,
    pub reason: String,
}

/// An exchange that has just been saved, as far as the device log cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedExchange {
    /// The payment method was cash.
    pub cash: bool,
    pub submission_id: String,
    pub document_ref: String,
    pub currency: String,
    /// The foreign amount.
    pub amount: Decimal,
    pub rate: Decimal,
    /// Key of the uploaded passport file, when there was one.
    pub passport_key: Option<String>,
    /// Key of the uploaded payment slip, when there was one.
    pub slip_key: Option<String>,
}
