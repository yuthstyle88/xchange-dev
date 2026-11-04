//! An audit line the kiosk asks to have written.

use crate::domain::kiosk::value_objects::cancel_reason::CancelReason;
use crate::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
use crate::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;

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
    /// `branch_currencies.display` was set for a currency and group (`docs/design/board-currencies.md` §3).
    CurrencyDisplaySet {
        group: String,
        display: String,
        from: String,
    },
    Cancelled {
        from: KioskStatus,
        reason: CancelReason,
    },
    /// The desk saved the request on the passport's customer, who is not the request's member
    /// (`kiosk-shared-phone.md` D13). The member stays on the request and its quote.
    CustomerDiffers {
        saved_customer: String,
        request_customer: String,
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
            Self::CurrencyConfigured { .. } | Self::CurrencyDisplaySet { .. } => {
                "KIOSK_CURRENCY_CONFIGURED"
            }
            Self::Cancelled { .. } => "KIOSK_CANCELLED",
            Self::CustomerDiffers { .. } => "KIOSK_CUSTOMER_DIFFERS",
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
