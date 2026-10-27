//! The shortfall a refused posting would have left (design `ledger-inventory.md` §5.1, XC-BE-GAPS-A row 1).
//!
//! `LedgerCashPostings` refuses a posting that takes a drawer, a safe or a position below zero with
//! `stockInsufficient` (§4.5). The refusal carries how far below zero each place would have ended, so the
//! caller that opens the `NEGATIVE_STOCK` request can put it in `proposed_payload` (§5.1). `AppError` holds
//! only a message, so the shortfall rides in the message after [`StockShortfall::MARKER`], written and read
//! only here.

use std::str::FromStr;

use pos_error::AppError;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xc_kernel::money::CurrencyCode;

const SENTENCE: &str =
    "stockInsufficient: the posting takes a drawer, a safe or a position below zero";

#[derive(Serialize, Deserialize)]
struct WireShortfall {
    currency: String,
    place: String,
    #[serde(rename = "shortBy")]
    short_by: String,
}

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
        match self {
            Self::Drawer => "DRAWER",
            Self::Safe => "SAFE",
            Self::Position => "POSITION",
        }
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

    fn wire(&self) -> Vec<WireShortfall> {
        self.lines
            .iter()
            .map(|l| WireShortfall {
                currency: l.currency.as_str().to_owned(),
                place: l.place.as_str().to_owned(),
                short_by: l.short_by.normalize().to_string(),
            })
            .collect()
    }

    /// The port's refusal: `stockInsufficient: <the §4.5 sentence>` + [`Self::MARKER`] + the lines as a
    /// compact JSON array of `{currency, place, shortBy}`, `shortBy` the normalized decimal as a string.
    pub fn refusal(&self) -> AppError {
        let json = serde_json::to_string(&self.wire()).expect("serialize shortfall");
        AppError::Domain(format!("{SENTENCE}{}{json}", Self::MARKER))
    }

    /// The shortfall a `stockInsufficient` message carries; empty for a message without [`Self::MARKER`]
    /// or with an array it cannot read.
    pub fn from_refusal(message: &str) -> Self {
        let Some((_, tail)) = message.split_once(Self::MARKER) else {
            return Self::default();
        };
        let Ok(raw) = serde_json::from_str::<Vec<WireShortfall>>(tail) else {
            return Self::default();
        };
        let mut lines = Vec::with_capacity(raw.len());
        for w in raw {
            let Ok(currency) = CurrencyCode::parse(&w.currency) else {
                return Self::default();
            };
            let place = match w.place.as_str() {
                "DRAWER" => ShortPlace::Drawer,
                "SAFE" => ShortPlace::Safe,
                "POSITION" => ShortPlace::Position,
                _ => return Self::default(),
            };
            let Ok(short_by) = Decimal::from_str(&w.short_by) else {
                return Self::default();
            };
            lines.push(Shortfall {
                currency,
                place,
                short_by: short_by.normalize(),
            });
        }
        Self { lines }
    }

    /// `proposed_payload` of the `NEGATIVE_STOCK` request:
    /// `{"shortfall": [{"currency": "USD", "place": "DRAWER", "shortBy": "50"}, …]}`.
    pub fn payload(&self) -> Value {
        json!({ "shortfall": self.wire() })
    }
}
