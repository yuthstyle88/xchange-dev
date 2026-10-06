//! An audit line the kiosk asks to have written.

use crate::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
use crate::domain::kiosk::value_objects::kiosk_quote::KioskQuote;

/// Who an audit line names when the customer confirmed at the kiosk.
pub const CUSTOMER_KIOSK: &str = "CUSTOMER-KIOSK";

/// What happened, with what v2 keeps beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KioskAudit {
    /// A customer confirmed a quote; the quote is kept with the line.
    Request {
        quote: KioskQuote,
    },
    Accepted,
    IdChecked,
    /// A saved transaction completed the request.
    Completed {
        document_ref: String,
    },
    /// A currency was switched for the kiosk; the rates are the texts that were sent.
    CurrencyConfigured {
        enabled: bool,
        buy: String,
        sell: String,
    },
}

impl KioskAudit {
    /// The kind v2 writes: `KIOSK_REQUEST`, …, `KIOSK_CURRENCY_CONFIGURED`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Request { .. } => KioskEventKind::Request.as_str(),
            Self::Accepted => KioskEventKind::Accepted.as_str(),
            Self::IdChecked => KioskEventKind::IdChecked.as_str(),
            Self::Completed { .. } => KioskEventKind::Completed.as_str(),
            Self::CurrencyConfigured { .. } => "KIOSK_CURRENCY_CONFIGURED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLine {
    /// A user name, or [`CUSTOMER_KIOSK`].
    pub actor: String,
    /// The request id, or the currency code for a switch.
    pub subject: String,
    pub what: KioskAudit,
}
