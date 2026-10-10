//! Kiosk currencies view DTOs for v3 (§7.3).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::{Rate, RateFreshnessDto};

/// One country where a currency is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_currencies.ts")]
pub struct KioskCountry {
    pub code: String,
    pub en: String,
    pub th: String,
}

/// KBank exchange rate reference information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_currencies.ts")]
pub struct KbankReference {
    pub source: String,
    pub published_at: String,
    #[ts(type = "number")]
    pub round: i64,
}

/// KBank rate comparison row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_currencies.ts")]
pub struct KbankComparison {
    pub label: String,
    pub bank_buy: Option<String>,
    pub shop_buy: Option<String>,
    pub buy_difference: Option<String>,
    pub bank_sell: Option<String>,
    pub shop_sell: Option<String>,
    pub sell_difference: Option<String>,
}

/// One currency in the kiosk currencies view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_currencies.ts")]
pub struct KioskCurrency {
    pub code: String,
    pub name: String,
    pub name_en: String,
    pub countries: Vec<KioskCountry>,
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
    pub unit_size: String,
    pub updated: String,
    pub partner_name: String,
    pub partner_cost: Option<Rate>,
    pub enabled: bool,
    pub kbank: Vec<KbankComparison>,
}

/// The `kiosk_currencies` view data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_currencies.ts")]
pub struct KioskCurrencies {
    pub currencies: Vec<KioskCurrency>,
    pub kbank: Option<KbankReference>,
    pub freshness: RateFreshnessDto,
}
