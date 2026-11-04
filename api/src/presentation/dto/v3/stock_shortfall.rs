//! Refusal details for `stockInsufficient` (design `ledger-inventory.md` §5.1.2; #6092).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_cash::domain::shortfall::{Shortfall, StockShortfall};

/// One shortfall line on a 409 `stockInsufficient` refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "stock_shortfall.ts")]
pub struct StockShortfallLine {
    pub currency: String,
    pub place: String,
    pub short_by: String,
}

/// `error.details` on a 409 `stockInsufficient` refusal: how far below zero each place would have ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "stock_shortfall.ts")]
pub struct StockShortfallDetails {
    pub lines: Vec<StockShortfallLine>,
}

impl From<&Shortfall> for StockShortfallLine {
    fn from(s: &Shortfall) -> Self {
        Self {
            currency: s.currency.as_str().to_owned(),
            place: s.place.as_str().to_owned(),
            short_by: s.short_by.normalize().to_string(),
        }
    }
}

impl From<Shortfall> for StockShortfallLine {
    fn from(s: Shortfall) -> Self {
        Self::from(&s)
    }
}

impl From<&StockShortfall> for StockShortfallDetails {
    fn from(s: &StockShortfall) -> Self {
        Self {
            lines: s.lines.iter().map(StockShortfallLine::from).collect(),
        }
    }
}

impl From<StockShortfall> for StockShortfallDetails {
    fn from(s: StockShortfall) -> Self {
        Self::from(&s)
    }
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    StockShortfallLine::export_all(&cfg)?;
    StockShortfallDetails::export_all(&cfg)?;
    Ok(())
}
