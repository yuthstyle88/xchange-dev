//! Rate policy domain entity and guard parsing.

use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;
use xc_kernel::money::{RatePercent, Side};

use crate::domain::draft::rate_engine_error::RateEngineError;

#[derive(Debug, Clone, PartialEq)]
pub struct GuardRange {
    pub min_pct: Decimal,
    pub max_pct: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RatePolicy {
    id: String,
    buy_adjust_pct: RatePercent,
    sell_adjust_pct: RatePercent,
    guards: Value,
}

impl RatePolicy {
    pub fn new(
        id: impl Into<String>,
        buy_adjust_pct: RatePercent,
        sell_adjust_pct: RatePercent,
        guards: &Value,
    ) -> Result<Self, RateEngineError> {
        Ok(Self {
            id: id.into(),
            buy_adjust_pct,
            sell_adjust_pct,
            guards: guards.clone(),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn buy_adjust_pct(&self) -> RatePercent {
        self.buy_adjust_pct
    }

    pub fn sell_adjust_pct(&self) -> RatePercent {
        self.sell_adjust_pct
    }

    pub fn guards(&self) -> &Value {
        &self.guards
    }

    pub fn guard_for(&self, currency: &str, side: Side) -> Option<GuardRange> {
        let side_key = match side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let guard_obj = self.guards.get(currency)?.get(side_key)?;
        let min_str = guard_obj.get("min_pct")?.as_str()?;
        let max_str = guard_obj.get("max_pct")?.as_str()?;
        let min_pct = Decimal::from_str(min_str).ok()?;
        let max_pct = Decimal::from_str(max_str).ok()?;
        Some(GuardRange { min_pct, max_pct })
    }
}
