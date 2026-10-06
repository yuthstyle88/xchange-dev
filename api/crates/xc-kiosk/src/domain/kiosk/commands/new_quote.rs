//! A quote asked for at the kiosk.

use crate::domain::kiosk::commands::quote_input::QuoteInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewQuote {
    pub input: QuoteInput,
    /// The id the request gets; the caller draws it (a lower-case UUID).
    pub new_id: String,
    /// The address the request came from, for the per-address limit.
    pub client_ip: String,
}
