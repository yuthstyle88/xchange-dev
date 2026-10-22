//! ReferenceProvider trait and parsed batch/line domain models (§2, §3.1).

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::rate::entities::rate_source::ReferenceRates;

pub trait ReferenceProvider: Send + Sync {
    fn provider(&self) -> &'static str;
    fn reference_type(&self) -> &'static str;
    fn row_key(&self, row: &Value) -> String;
    fn parse(&self, rates: &ReferenceRates) -> Result<ParsedBatch, RateEngineError>;

    /// True when an answer identical to an earlier batch **confirms** that batch: the writer moves its `fetched_at` to
    /// this fetch instead of returning it untouched, so `fetched_at` means "last confirmed" (pricing-tiers.md §4).
    /// BoT keeps `false` (its behaviour is unchanged); the market feeds return `true`.
    fn confirms_on_repeat(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedBatch {
    pub source_date: NaiveDate,
    pub source_published_at: Option<DateTime<Utc>>,
    pub complete: bool,
    pub lines: Vec<ParsedLine>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedLine {
    pub currency: String,
    pub instrument_key: Option<String>,
    pub value_tier: Option<String>,
    pub unit_size: Decimal,
    pub base_buy: Option<Decimal>,
    pub base_sell: Option<Decimal>,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
    pub raw: Value,
}
