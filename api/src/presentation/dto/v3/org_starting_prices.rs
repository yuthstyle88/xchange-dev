//! SP-1 (#1585, rates design §12.6, §12.10): wire types for org starting prices
//! (`POST /api/v3/commands/org_starting_price_save` and `org_starting_prices` view).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::org_view::OrgFilter;

/// One price entry in `OrgStartingPriceSaveInput`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "org_starting_price_save.ts")]
pub struct OrgStartingPriceEntryInput {
    pub currency: String,
    #[serde(default)]
    pub buy: Option<String>,
    #[serde(default)]
    pub sell: Option<String>,
}

/// The body of `POST /api/v3/commands/org_starting_price_save`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "org_starting_price_save.ts")]
pub struct OrgStartingPriceSaveInput {
    pub prices: Vec<OrgStartingPriceEntryInput>,
    pub reason: String,
}

/// An unpublished branch that could not be priced automatically.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "org_starting_price_save.ts")]
pub struct UnpricedBranch {
    pub branch: String,
    pub reason: String,
}

/// The output of `POST /api/v3/commands/org_starting_price_save`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "org_starting_price_save.ts")]
pub struct OrgStartingPriceSaveOutput {
    pub unpriced: Vec<UnpricedBranch>,
}

/// One starting price record in the `org_starting_prices` view.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "org_starting_prices.ts")]
pub struct OrgStartingPriceItem {
    pub currency: String,
    pub version: i32,
    pub buy: Option<String>,
    pub sell: Option<String>,
    pub entered_by: String,
    pub entered_at: String,
    pub reason: String,
}

/// Filter for the `org_starting_prices` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "org_starting_prices.ts")]
pub struct OrgStartingPricesFilter;

impl OrgFilter for OrgStartingPricesFilter {
    fn parse(_query: &BTreeMap<String, String>) -> Result<Self, V3Error> {
        Ok(Self)
    }
}

pub fn export_command_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    OrgStartingPriceEntryInput::export_all(&cfg)?;
    OrgStartingPriceSaveInput::export_all(&cfg)?;
    UnpricedBranch::export_all(&cfg)?;
    OrgStartingPriceSaveOutput::export_all(&cfg)?;
    Ok(())
}

pub fn export_view_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    OrgStartingPriceItem::export_all(&cfg)?;
    OrgStartingPricesFilter::export_all(&cfg)?;
    Ok(())
}
