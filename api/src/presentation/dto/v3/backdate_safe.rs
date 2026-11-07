//! The `backdate_safe` view's wire types (`docs/design/backdated-keying.md` §16.3, F56): the SAFE balance of the branch
//! a backdated sale posts against, so the keyer sees a shortfall before filling in a row, not after the server refuses it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One currency held in the branch's SAFE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "backdate_safe.ts")]
pub struct BackdateSafeLine {
    /// ISO code, trimmed, e.g. `"THB"`.
    pub currency: String,
    /// The balance written to the currency's minor unit, as `/staff` writes it: THB `150000.00`, JPY `5000`.
    pub amount: String,
}

/// Every currency with a non-zero SAFE balance, ordered by code. Empty when the SAFE holds nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "backdate_safe.ts")]
pub struct BackdateSafeData {
    pub balances: Vec<BackdateSafeLine>,
}
