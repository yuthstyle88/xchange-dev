//! The shortfall a refused posting would have left (design `ledger-inventory.md` §5.1, XC-BE-GAPS-A row 1).
//!
//! `LedgerCashPostings` refuses a posting that takes a drawer, a safe or a position below zero with
//! `stockInsufficient` (§4.5). The refusal carries how far below zero each place would have ended, so the
//! caller that opens the `NEGATIVE_STOCK` request can put it in `proposed_payload` (§5.1). `AppError` holds
//! only a message, so the shortfall rides in the message after [`StockShortfall::MARKER`], written and read
//! only here.

use pos_error::AppError;
use rust_decimal::Decimal;
use serde_json::Value;
use xc_kernel::money::CurrencyCode;

/// The kind of place a posting would take below zero (§4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortPlace {
    Drawer,
    Safe,
    Position,
}

impl ShortPlace {
    /// `DRAWER`, `SAFE` or `POSITION`.
    pub fn as_str(self) -> &'static str {
        todo!("XC-BE-GAPS-A row 1")
    }
}

/// One currency at one kind of place: `short_by` is how far below zero it would end (> 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortfall {
    pub currency: CurrencyCode,
    pub place: ShortPlace,
    pub short_by: Decimal,
}

/// Every place a refused posting would have taken below zero, sorted by currency code, then place
/// (`Drawer`, `Safe`, `Position`). Empty when the refusal carried none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StockShortfall {
    pub lines: Vec<Shortfall>,
}

impl StockShortfall {
    /// What separates the refusal's sentence from the shortfall it carries.
    pub const MARKER: &'static str = "; shortfall=";

    /// The port's refusal: `stockInsufficient: <the §4.5 sentence>` + [`Self::MARKER`] + the lines as a
    /// compact JSON array of `{currency, place, shortBy}`, `shortBy` the normalized decimal as a string.
    pub fn refusal(&self) -> AppError {
        todo!("XC-BE-GAPS-A row 1")
    }

    /// The shortfall a `stockInsufficient` message carries; empty for a message without [`Self::MARKER`]
    /// or with an array it cannot read.
    pub fn from_refusal(message: &str) -> Self {
        let _ = message;
        todo!("XC-BE-GAPS-A row 1")
    }

    /// `proposed_payload` of the `NEGATIVE_STOCK` request:
    /// `{"shortfall": [{"currency": "USD", "place": "DRAWER", "shortBy": "50"}, …]}`.
    pub fn payload(&self) -> Value {
        todo!("XC-BE-GAPS-A row 1")
    }
}
