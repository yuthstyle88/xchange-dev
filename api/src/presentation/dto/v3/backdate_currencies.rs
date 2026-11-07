//! The `backdate_currencies` view's wire types (`docs/design/backdated-keying.md` §15.2, F46): every currency the
//! backdated form may key, with the unit its rate is quoted per. It is the same table `BackdateSale::run_in` prices
//! from (`currencies`), so the form and the server cannot disagree on a unit.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One currency the backdated form may key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "backdate_currencies.ts")]
pub struct BackdateCurrencyRow {
    /// ISO code, trimmed, e.g. `"JPY"`.
    pub code: String,
    /// `currencies.unit_size` with trailing zeros removed: `"1"`, `"100"`, `"1000"`, never `"1.0000"`.
    pub unit_size: String,
    /// `currencies.minor_unit`: the decimal places of a foreign amount.
    pub minor_unit: u8,
}

/// Every row of `currencies` except THB, ordered by `code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "backdate_currencies.ts")]
pub struct BackdateCurrenciesData {
    pub currencies: Vec<BackdateCurrencyRow>,
}
